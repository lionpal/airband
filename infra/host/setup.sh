#!/bin/bash
# Put the relay that was shipped into service on this server. Run as root, from the folder the shipment was
# unpacked into, by the `airband-relay-deploy` document (infra/cdk.mjs) at each deploy and on a new server's
# first boot. Safe to run again: it changes only what differs, and restarts only what changed.
#
# In the folder: airband-relay (the binary), start, airband-relay.service, caddy.service, Caddyfile, this file.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)

session=$(curl -sf -X PUT http://169.254.169.254/latest/api/token -H 'X-aws-ec2-metadata-token-ttl-seconds: 60')
region=$(curl -sf -H "X-aws-ec2-metadata-token: $session" http://169.254.169.254/latest/meta-data/placement/region)

# half a gigabyte of memory is enough for the relay and Caddy, and not always for the system's own updates
if ! swapon --show=NAME --noheadings | grep -qx /swapfile; then
  [ -f /swapfile ] || dd if=/dev/zero of=/swapfile bs=1M count=768 status=none
  chmod 600 /swapfile
  swapon /swapfile 2>/dev/null || { mkswap /swapfile >/dev/null && swapon /swapfile; }
  grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi

id -u airband >/dev/null 2>&1 || useradd --system --no-create-home --shell /sbin/nologin airband
id -u caddy >/dev/null 2>&1 || useradd --system --home-dir /var/lib/caddy --create-home --shell /sbin/nologin caddy
install -d -m 755 /opt/airband /etc/airband /etc/caddy /etc/systemd/journald.conf.d
install -d -m 750 -o caddy -g caddy /var/lib/caddy
# the logs stay on this disk, and only so much of it
printf '[Journal]\nSystemMaxUse=200M\n' > /etc/systemd/journald.conf.d/airband.conf

# put a file in place if it differs from what is there; says so by succeeding
place() { # <from> <to> <mode>
  cmp -s "$1" "$2" && return 1
  install -m "$3" "$1" "$2.new" && mv -f "$2.new" "$2"
}

# settings that are not secret, as the infrastructure has them now
aws ssm get-parameter --region "$region" --name /airband/relay-env --query Parameter.Value --output text > "$here/relay.env"
grep -q '^AIRBAND_RELAY_DYNAMO=' "$here/relay.env"
relay=0; caddy=0; units=0
place "$here/relay.env" /etc/airband/relay.env 644 && { relay=1; caddy=1; }
. /etc/airband/relay.env

if [ ! -x /usr/local/bin/caddy ]; then
  aws s3 cp --only-show-errors "s3://$SHIP_BUCKET/caddy/caddy-linux-arm64" "$here/caddy"
  install -m 755 "$here/caddy" /usr/local/bin/caddy
  caddy=1
fi
place "$here/Caddyfile" /etc/caddy/Caddyfile 644 && caddy=1
place "$here/caddy.service" /etc/systemd/system/caddy.service 644 && { caddy=2; units=1; }
place "$here/airband-relay.service" /etc/systemd/system/airband-relay.service 644 && { relay=1; units=1; }
place "$here/start" /opt/airband/start 755 && relay=1
[ -f /opt/airband/airband-relay ] && cp -pf /opt/airband/airband-relay /opt/airband/airband-relay.before
place "$here/airband-relay" /opt/airband/airband-relay 755 && relay=1
[ "$units" = 1 ] && systemctl daemon-reload && systemctl restart systemd-journald
systemctl enable --quiet airband-relay caddy

answers() { for _ in $(seq 1 40); do curl -sf -o /dev/null http://127.0.0.1:"$PORT"/healthz && return 0; sleep 0.5; done; return 1; }

if [ "$relay" = 1 ] || ! systemctl is-active --quiet airband-relay; then
  systemctl restart airband-relay
  if ! answers; then
    echo "the new relay does not answer:"; journalctl -u airband-relay -n 30 --no-pager || true
    # back to the one that was running, if there was one
    if [ -f /opt/airband/airband-relay.before ]; then
      mv -f /opt/airband/airband-relay.before /opt/airband/airband-relay
      systemctl restart airband-relay && answers && echo "the relay from before is running again"
    fi
    exit 1
  fi
  echo "the relay was restarted and answers"
else
  echo "the relay is unchanged and was left running"
fi

# a Caddyfile that Caddy would refuse stops the deploy here, with the old settings still in use
(set -a; . /etc/airband/relay.env; set +a; /usr/local/bin/caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile 2>&1 | tail -3)
if ! systemctl is-active --quiet caddy || [ "$caddy" = 2 ]; then
  systemctl restart caddy
  echo "Caddy was started"
elif [ "$caddy" = 1 ]; then
  systemctl reload caddy
  echo "Caddy took its new settings"
fi
journalctl -u airband-relay -n 6 --no-pager -o cat || true
