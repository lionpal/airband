// The Airband relay and homepage. Three stacks:
//
//   AirbandRelayUs  where relay.airband.dev runs: one small server in the United States with DynamoDB behind it.
//   AirbandRelay    where it ran before, and can again: a Fargate task in Tokyo behind an existing load balancer.
//                   Nothing deploys it any more unless asked to (see .github/workflows/deploy-relay.yml).
//   AirbandSite     the homepage.
//
// AirbandRelay: the relay as one small Fargate task behind an existing load balancer, keeping its
// channel list in an existing database cluster. Nothing new is rented but the task itself.
//
// Nothing that is already there is replaced. This stack ADDS a
// certificate and one host-header rule to the production HTTPS listener, a target group, a small ECS service in the
// same VPC's public subnets, one ingress rule on the database's security group, and a DNS record. The listener's
// default action is never referenced, so whatever it served before is served exactly as before. Deleting this stack removes only
// what it added (the `airband` database on the cluster stays; drop it by hand if it is ever unwanted).
//
// Before the first deploy:
//   1. airband.dev must be delegated to this account's hosted zone (the registrar's name servers set to the zone's
//      four), or the certificate never validates and the deploy waits until it times out.
//   2. the admission key must exist:  aws ssm put-parameter --name /airband/relay-admission --type SecureString --value <key>
// Deploy: the GitHub Actions (.github/workflows/deploy-relay.yml and deploy-site.yml), one per stack.
// By hand, if the Action is down:  cd infra && npm ci && npx cdk deploy AirbandRelay --require-approval never
// (a by-hand deploy builds the image locally and needs several GB of free disk for Docker)
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { App, CfnOutput, Duration, RemovalPolicy, Stack, Tags } from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as acm from 'aws-cdk-lib/aws-certificatemanager';
import * as cloudfront from 'aws-cdk-lib/aws-cloudfront';
import * as origins from 'aws-cdk-lib/aws-cloudfront-origins';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3deploy from 'aws-cdk-lib/aws-s3-deployment';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import { Platform } from 'aws-cdk-lib/aws-ecr-assets';
import * as ecs from 'aws-cdk-lib/aws-ecs';
import * as elbv2 from 'aws-cdk-lib/aws-elasticloadbalancingv2';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as route53 from 'aws-cdk-lib/aws-route53';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import * as targets from 'aws-cdk-lib/aws-route53-targets';
import * as ssm from 'aws-cdk-lib/aws-ssm';

const HERE = dirname(fileURLToPath(import.meta.url));
// Everything that belongs to the AWS account this is deployed into is kept out of the repository: which account,
// which domain, and the network, load balancer and database the relay sits beside. It is one JSON object in
// AIRBAND_INFRA, with the keys in infra.example.json. The deploy workflows get it from a GitHub Actions secret;
// to deploy by hand, export it first.
const INFRA = JSON.parse(process.env.AIRBAND_INFRA || 'null');
if (!INFRA) throw new Error('Set AIRBAND_INFRA to a JSON object with the keys in infra/infra.example.json.');
const of = (key) => {
  if (INFRA[key] === undefined) throw new Error(`AIRBAND_INFRA has no "${key}" (see infra/infra.example.json).`);
  return INFRA[key];
};
const ACCOUNT = of('account'), REGION = of('region');
// "speechRegion" is optional: with it, the relay lends its speech service (see the task's secrets below)
const SPEECH_REGION = INFRA.speechRegion;
// Optional: with AIRBAND_APNS set to {"keyId","teamId","topic"} the relay can wake iPhones through Apple's push
// service. None of the three is secret. The signing key itself is (see the task's secrets below). The deploy
// workflow takes it from the repository variable AIRBAND_APNS.
const APNS = JSON.parse(process.env.AIRBAND_APNS || 'null');
const ZONE_NAME = of('zoneName'), ZONE_ID = of('zoneId'), DOMAIN = of('domain');
// an existing load balancer and its network, by id: the relay is one more rule on its listener
const VPC_ID = of('vpcId'), PUBLIC_SUBNETS = of('publicSubnets'), ZONES = of('zones');
const ALB_ARN = of('albArn'), ALB_DNS = of('albDns'), ALB_ZONE = of('albZone'), ALB_SG = of('albSg'), LISTENER_ARN = of('listenerArn');
// An existing Postgres cluster. The relay keeps its own database on it, which it creates on first start
// (crates/airband-relay/src/store.rs), signing in with the cluster's secret.
const DB_HOST = of('dbHost'), DB_NAME = of('dbName'), AURORA_SG_ID = of('auroraSgId'), DB_SECRET_ARN = of('dbSecretArn');
const RULE_PRIORITY = of('rulePriority'); // a priority no other rule on the listener uses
const PORT = 7474;
// The server in the United States: { region, zone, ami, githubSubject }. `ami` is an Amazon Linux 2023 arm64
// image in that region, named outright so a newer image never replaces the server behind our back
// (aws ssm get-parameter --name /aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-arm64).
// `githubSubject` is what GitHub's token says a run on this repository's main branch is (the `sub` claim).
const US = INFRA.us;

class AirbandRelay extends Stack {
  constructor(scope, id, props) {
    super(scope, id, props);
    const vpc = ec2.Vpc.fromVpcAttributes(this, 'Vpc', { vpcId: VPC_ID, availabilityZones: ZONES, publicSubnetIds: PUBLIC_SUBNETS });
    const zone = route53.HostedZone.fromHostedZoneAttributes(this, 'Zone', { hostedZoneId: ZONE_ID, zoneName: ZONE_NAME });
    const cert = new acm.Certificate(this, 'Cert', { domainName: DOMAIN, validation: acm.CertificateValidation.fromDns(zone) });

    const dbSecret = secretsmanager.Secret.fromSecretCompleteArn(this, 'DbSecret', DB_SECRET_ARN);
    const cluster = new ecs.Cluster(this, 'Cluster', { vpc });
    // x86, like the other services on this load balancer: it is what GitHub's standard build machines produce natively
    const task = new ecs.FargateTaskDefinition(this, 'Task', { cpu: 256, memoryLimitMiB: 512, runtimePlatform: { cpuArchitecture: ecs.CpuArchitecture.X86_64, operatingSystemFamily: ecs.OperatingSystemFamily.LINUX } });
    task.addContainer('relay', {
      image: ecs.ContainerImage.fromAsset(join(HERE, '..'), { platform: Platform.LINUX_AMD64 }),
      portMappings: [{ containerPort: PORT }],
      environment: { PORT: String(PORT), DB_HOST, DB_PORT: '5432', DB_NAME, ...(SPEECH_REGION ? { AZURE_SPEECH_REGION: SPEECH_REGION } : {}),
        ...(APNS ? { APNS_KEY_ID: APNS.keyId, APNS_TEAM_ID: APNS.teamId, APNS_TOPIC: APNS.topic } : {}) },
      secrets: {
        // optional: a speech service this relay lends to towers that have none. The key is written once by hand
        // (aws ssm put-parameter --name /airband/speech-key --type SecureString --value <key>) and never in the repo
        ...(SPEECH_REGION ? { AZURE_SPEECH_KEY: ecs.Secret.fromSsmParameter(ssm.StringParameter.fromSecureStringParameterAttributes(this, 'SpeechKey', { parameterName: '/airband/speech-key' })) } : {}),
        // optional: Apple's push signing key, the .p8 file's text, written once by hand and never in the repo
        // (aws ssm put-parameter --name /airband/apns-key --type SecureString --value file://AuthKey_….p8)
        ...(APNS ? { APNS_KEY: ecs.Secret.fromSsmParameter(ssm.StringParameter.fromSecureStringParameterAttributes(this, 'ApnsKey', { parameterName: '/airband/apns-key' })) } : {}),
        DB_USER: ecs.Secret.fromSecretsManager(dbSecret, 'username'), DB_PASS: ecs.Secret.fromSecretsManager(dbSecret, 'password'),
        // without this anyone on the internet could open channels here; written once by hand, never in the repo
        AIRBAND_RELAY_ADMISSION: ecs.Secret.fromSsmParameter(ssm.StringParameter.fromSecureStringParameterAttributes(this, 'Admission', { parameterName: '/airband/relay-admission' })),
      },
      logging: ecs.LogDrivers.awsLogs({ streamPrefix: 'relay', logRetention: logs.RetentionDays.TWO_WEEKS }),
    });
    // ONE task, and never two at once: requests in flight are held in the task's memory, so a tower pulling from one
    // task would never see a phone's call that landed on another. A deploy therefore stops the old task before the
    // new one starts: about a minute during which phones show "no tower" and retry.
    const service = new ecs.FargateService(this, 'Service', { cluster, taskDefinition: task, desiredCount: 1, assignPublicIp: true, vpcSubnets: { subnets: vpc.publicSubnets }, minHealthyPercent: 0, maxHealthyPercent: 100, circuitBreaker: { rollback: true } });

    const tg = new elbv2.ApplicationTargetGroup(this, 'Tg', { vpc, port: PORT, protocol: elbv2.ApplicationProtocol.HTTP, targetType: elbv2.TargetType.IP, targets: [service], deregistrationDelay: Duration.seconds(15),
      healthCheck: { path: '/healthz', interval: Duration.seconds(20), healthyThresholdCount: 2 } });
    // both flags: mutable alone wrote no egress rule on the load balancer's group
    const albSg = ec2.SecurityGroup.fromSecurityGroupId(this, 'AlbSg', ALB_SG, { mutable: true, allowAllOutbound: false });
    const alb = elbv2.ApplicationLoadBalancer.fromApplicationLoadBalancerAttributes(this, 'Alb', { loadBalancerArn: ALB_ARN, loadBalancerDnsName: ALB_DNS, loadBalancerCanonicalHostedZoneId: ALB_ZONE, securityGroupId: ALB_SG, vpc });
    const listener = elbv2.ApplicationListener.fromApplicationListenerAttributes(this, 'Listener', { listenerArn: LISTENER_ARN, securityGroup: albSg });
    new elbv2.ApplicationListenerCertificate(this, 'ListenerCert', { listener, certificates: [cert] });
    new elbv2.ApplicationListenerRule(this, 'HostRule', { listener, priority: RULE_PRIORITY, conditions: [elbv2.ListenerCondition.hostHeaders([DOMAIN])], action: elbv2.ListenerAction.forward([tg]) });
    service.connections.allowFrom(albSg, ec2.Port.tcp(PORT), of('albIngressNote'));
    // let the task reach the shared database, the way the cluster's other services are let in
    const auroraSg = ec2.SecurityGroup.fromSecurityGroupId(this, 'AuroraSg', AURORA_SG_ID, { mutable: true });
    auroraSg.addIngressRule(ec2.Peer.securityGroupId(service.connections.securityGroups[0].securityGroupId), ec2.Port.tcp(5432), 'Airband relay task to the shared Aurora cluster');
    new route53.ARecord(this, 'Alias', { zone, recordName: 'relay', target: route53.RecordTarget.fromAlias(new targets.LoadBalancerTarget(alb)) });
    new CfnOutput(this, 'Url', { value: `https://${DOMAIN}` });
  }
}
// relay.airband.dev in the United States: one small arm64 server with a fixed address, and DynamoDB for what the
// relay keeps. No load balancer and no database server, which is the point: the server, its disk and its address
// are all there is to pay for by the month.
//
// On the server: Caddy answers on 443 with a certificate it gets and renews itself, proving the names by a DNS
// record (so the certificate can exist before the name points here), and hands requests to the relay on
// 127.0.0.1. The relay runs as its own user under systemd. Nobody can sign in from the internet: there is no
// SSH; `aws ssm start-session` is the way in. The server's role can read and write its three tables, read its
// own settings and secrets, fetch what is shipped to it, and write the two DNS records Caddy proves itself with.
//
// Shipping: the workflow builds the relay for arm64, puts it with the files in infra/host/ in the bucket here,
// and asks the server to run the `airband-relay-deploy` document, which fetches that and runs its setup.sh.
// The role the workflow uses for this can do only those two things. A new server does the same on first boot.
//
// Before the first deploy, once, by hand (none of it is in the repository):
//   1. the three secrets, in this stack's region, as SecureString parameters:
//      /airband/relay-admission, /airband/speech-key, /airband/apns-key
//   2. a Caddy built with the Route 53 module, in the bucket as caddy/caddy-linux-arm64
//      (https://caddyserver.com/api/download?os=linux&arch=arm64&p=github.com/caddy-dns/route53)
// The name itself (the A record for relay.airband.dev) is not made here while the Tokyo stack still owns it:
// it is pointed at this server's address by hand. This stack makes relay-us.<zone>, which always points here.
class AirbandRelayUs extends Stack {
  constructor(scope, id, props) {
    super(scope, id, props);
    Tags.of(this).add('app', 'airband');
    const zone = route53.HostedZone.fromHostedZoneAttributes(this, 'Zone', { hostedZoneId: ZONE_ID, zoneName: ZONE_NAME });
    const names = [DOMAIN, `relay-us.${ZONE_NAME}`];
    const prefix = 'airband-relay';

    // what the relay keeps (crates/airband-relay/src/store.rs says what is in each). Paid for by the request.
    const table = (name, keys, more = {}) => new dynamodb.Table(this, name, { tableName: `${prefix}-${name.toLowerCase()}`, ...keys, billingMode: dynamodb.BillingMode.PAY_PER_REQUEST, removalPolicy: RemovalPolicy.RETAIN, ...more });
    const text = (name) => ({ name, type: dynamodb.AttributeType.STRING });
    const tables = [
      table('Channels', { partitionKey: text('channel') }),
      table('Held', { partitionKey: text('channel'), sortKey: text('k') }, { timeToLiveAttribute: 'expires' }),
      // the one thing here that cannot be made again: it can be put back as it was at any moment of the last five weeks
      table('Waitlist', { partitionKey: text('email') }, { pointInTimeRecoverySpecification: { pointInTimeRecoveryEnabled: true } }),
    ];

    const ship = new s3.Bucket(this, 'Ship', { bucketName: `airband-relay-ship-${this.account}-${this.region}`, blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL, enforceSSL: true, versioned: true,
      lifecycleRules: [{ noncurrentVersionExpiration: Duration.days(30) }] });

    // a network of its own with one public subnet: nothing to share with, and nothing that costs by the hour
    const vpc = new ec2.Vpc(this, 'Vpc', { ipAddresses: ec2.IpAddresses.cidr('10.74.0.0/24'), availabilityZones: [US.zone], natGateways: 0, restrictDefaultSecurityGroup: false,
      subnetConfiguration: [{ name: 'public', subnetType: ec2.SubnetType.PUBLIC, cidrMask: 26 }] });
    const sg = new ec2.SecurityGroup(this, 'Sg', { vpc, description: 'airband relay: HTTPS in, anything out', allowAllOutbound: true });
    sg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(443), 'HTTPS to the relay');

    const role = new iam.Role(this, 'HostRole', { assumedBy: new iam.ServicePrincipal('ec2.amazonaws.com'), description: 'The airband relay server',
      managedPolicies: [iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSSMManagedInstanceCore')] });
    role.addToPolicy(new iam.PolicyStatement({ actions: ['dynamodb:GetItem', 'dynamodb:PutItem', 'dynamodb:UpdateItem', 'dynamodb:DeleteItem', 'dynamodb:Query', 'dynamodb:Scan'], resources: tables.map((t) => t.tableArn) }));
    const settings = ['relay-env', 'relay-admission', 'speech-key', 'apns-key'].map((n) => `arn:aws:ssm:${this.region}:${this.account}:parameter/airband/${n}`);
    role.addToPolicy(new iam.PolicyStatement({ actions: ['ssm:GetParameter', 'ssm:GetParameters'], resources: settings }));
    ship.grantRead(role);
    // the two records Caddy proves the names with, and nothing else in the zone
    role.addToPolicy(new iam.PolicyStatement({ actions: ['route53:ChangeResourceRecordSets'], resources: [zone.hostedZoneArn],
      conditions: { 'ForAllValues:StringEquals': { 'route53:ChangeResourceRecordSetsNormalizedRecordNames': names.map((n) => `_acme-challenge.${n}`), 'route53:ChangeResourceRecordSetsRecordTypes': ['TXT'] } } }));
    role.addToPolicy(new iam.PolicyStatement({ actions: ['route53:ListResourceRecordSets'], resources: [zone.hostedZoneArn] }));
    role.addToPolicy(new iam.PolicyStatement({ actions: ['route53:GetChange'], resources: ['arn:aws:route53:::change/*'] }));

    // everything the server is told that is not a secret: read by setup.sh at each deploy
    const env = { AWS_REGION: this.region, AIRBAND_RELAY_DYNAMO: prefix, HOST: '127.0.0.1', PORT: String(PORT), SHIP_BUCKET: ship.bucketName, RELAY_NAMES: `"${names.join(', ')}"`, ZONE_ID,
      ...(SPEECH_REGION ? { AZURE_SPEECH_REGION: SPEECH_REGION } : {}), ...(APNS ? { APNS_KEY_ID: APNS.keyId, APNS_TEAM_ID: APNS.teamId, APNS_TOPIC: APNS.topic } : {}) };
    new ssm.StringParameter(this, 'Env', { parameterName: '/airband/relay-env', stringValue: Object.entries(env).map(([k, v]) => `${k}=${v}`).join('\n') + '\n' });

    // fetch what was shipped and run its setup: what a deploy asks of the server, and what a new server does by itself
    const fetchAndSetUp = ['set -euo pipefail', 'd=$(mktemp -d)', `aws s3 cp --only-show-errors s3://${ship.bucketName}/relay/relay.tar.gz "$d/relay.tar.gz"`, 'tar -xzf "$d/relay.tar.gz" -C "$d"', 'bash "$d/setup.sh"', 'rm -rf "$d"'];
    const document = new ssm.CfnDocument(this, 'Deploy', { name: 'airband-relay-deploy', documentType: 'Command', documentFormat: 'JSON', updateMethod: 'NewVersion',
      content: { schemaVersion: '2.2', description: 'Fetch the airband relay that was shipped and put it in service', mainSteps: [{ action: 'aws:runShellScript', name: 'deploy', inputs: { timeoutSeconds: '600', runCommand: fetchAndSetUp } }] } });

    const userData = ec2.UserData.forLinux();
    userData.addCommands('# nothing to do until something has been shipped', ...fetchAndSetUp.slice(1, 3).map((c, i) => (i === 1 ? `${c} || exit 0` : c)), ...fetchAndSetUp.slice(3));
    // t4g.nano: two arm64 cores' worth of bursts and half a gigabyte, of which the relay and Caddy use about a tenth
    const host = new ec2.Instance(this, 'Host', { vpc, vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC }, instanceType: new ec2.InstanceType('t4g.nano'), machineImage: ec2.MachineImage.genericLinux({ [this.region]: US.ami }),
      role, securityGroup: sg, requireImdsv2: true, userData, blockDevices: [{ deviceName: '/dev/xvda', volume: ec2.BlockDeviceVolume.ebs(8, { volumeType: ec2.EbsDeviceVolumeType.GP3, encrypted: true }) }] });
    Tags.of(host).add('Name', 'airband-relay');
    const ip = new ec2.CfnEIP(this, 'Ip', { domain: 'vpc' });
    new ec2.CfnEIPAssociation(this, 'IpOnHost', { allocationId: ip.attrAllocationId, instanceId: host.instanceId });
    new route53.ARecord(this, 'UsName', { zone, recordName: 'relay-us', target: route53.RecordTarget.fromIpAddresses(ip.attrPublicIp), ttl: Duration.seconds(60) });

    // what the deploy workflow becomes to ship: it can put the relay in the bucket and ask this one server to run
    // this one document. It cannot run anything else there.
    const deployer = new iam.Role(this, 'Deployer', { roleName: 'airband-relay-us-deploy', description: 'GitHub Actions: ship the airband relay to its server',
      assumedBy: new iam.WebIdentityPrincipal(`arn:aws:iam::${this.account}:oidc-provider/token.actions.githubusercontent.com`, { StringEquals: { 'token.actions.githubusercontent.com:aud': 'sts.amazonaws.com', 'token.actions.githubusercontent.com:sub': US.githubSubject } }) });
    deployer.addToPolicy(new iam.PolicyStatement({ actions: ['s3:PutObject'], resources: [ship.arnForObjects('relay/*')] }));
    deployer.addToPolicy(new iam.PolicyStatement({ actions: ['ssm:SendCommand'], resources: [`arn:aws:ssm:${this.region}:${this.account}:document/${document.name}`] }));
    deployer.addToPolicy(new iam.PolicyStatement({ actions: ['ssm:SendCommand'], resources: [`arn:aws:ec2:${this.region}:${this.account}:instance/*`], conditions: { StringEquals: { 'ssm:resourceTag/Name': 'airband-relay' } } }));
    deployer.addToPolicy(new iam.PolicyStatement({ actions: ['ssm:ListCommandInvocations'], resources: ['*'] }));

    new CfnOutput(this, 'Address', { value: ip.attrPublicIp });
    new CfnOutput(this, 'Url', { value: `https://${names[1]}` });
  }
}

// airband.dev: the homepage. One static page (site/index.html) in a private bucket behind a CDN. It is its own
// stack so that changing the page never restarts the relay: the relay's deploy drops every phone for a minute, and
// a copy change should not. The page's waiting-list form posts to the relay (POST /v1/waitlist), which keeps the
// addresses in its database; nothing here stores anything.
//
// In us-east-1 because a CDN certificate has to be issued there. The zone is the same one, in the same account.
class AirbandSite extends Stack {
  constructor(scope, id, props) {
    super(scope, id, props);
    const zone = route53.HostedZone.fromHostedZoneAttributes(this, 'Zone', { hostedZoneId: ZONE_ID, zoneName: ZONE_NAME });
    const names = [ZONE_NAME, `www.${ZONE_NAME}`];
    const cert = new acm.Certificate(this, 'Cert', { domainName: names[0], subjectAlternativeNames: [names[1]], validation: acm.CertificateValidation.fromDns(zone) });
    const bucket = new s3.Bucket(this, 'Pages', { blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL, enforceSSL: true });
    // What the page may do, said to the browser: run its own script and styles, load its one font, talk to the
    // relay's sign-up address and nothing else, and never be shown inside another site's frame.
    const csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; "
      + "connect-src https://relay.airband.dev; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
    const headers = new cloudfront.ResponseHeadersPolicy(this, 'Headers', {
      securityHeadersBehavior: {
        contentSecurityPolicy: { contentSecurityPolicy: csp, override: true },
        strictTransportSecurity: { accessControlMaxAge: Duration.days(365), includeSubdomains: false, override: true },
        contentTypeOptions: { override: true },
        frameOptions: { frameOption: cloudfront.HeadersFrameOption.DENY, override: true },
        referrerPolicy: { referrerPolicy: cloudfront.HeadersReferrerPolicy.STRICT_ORIGIN_WHEN_CROSS_ORIGIN, override: true },
      },
    });
    const cdn = new cloudfront.Distribution(this, 'Cdn', {
      comment: 'airband.dev homepage',
      defaultBehavior: { origin: origins.S3BucketOrigin.withOriginAccessControl(bucket), viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS, compress: true, responseHeadersPolicy: headers,
        allowedMethods: cloudfront.AllowedMethods.ALLOW_GET_HEAD },
      minimumProtocolVersion: cloudfront.SecurityPolicyProtocol.TLS_V1_2_2021,
      defaultRootObject: 'index.html', domainNames: names, certificate: cert,
      // North America and Europe only: the cheapest class, and where the people who will read this are
      priceClass: cloudfront.PriceClass.PRICE_CLASS_100,
    });
    // five minutes: a new version of the page is what people see soon after a deploy, without a cache to chase.
    // Publishing removes whatever in the bucket is not in site/, except releases/: the built `airband` binaries
    // that Homebrew downloads (https://airband.dev/releases/<version>/…) are put there by the Release workflow.
    new s3deploy.BucketDeployment(this, 'Publish', { sources: [s3deploy.Source.asset(join(HERE, '..', 'site'))], destinationBucket: bucket, distribution: cdn, distributionPaths: ['/*'], cacheControl: [s3deploy.CacheControl.fromString('public, max-age=300')], exclude: ['releases/*'] });
    for (const [label, recordName] of [['Apex', undefined], ['Www', 'www']]) {
      const target = route53.RecordTarget.fromAlias(new targets.CloudFrontTarget(cdn));
      new route53.ARecord(this, `${label}A`, { zone, recordName, target });
      new route53.AaaaRecord(this, `${label}Aaaa`, { zone, recordName, target });
    }
    new CfnOutput(this, 'Url', { value: `https://${ZONE_NAME}` });
  }
}

const app = new App();
if (US) new AirbandRelayUs(app, 'AirbandRelayUs', { env: { account: ACCOUNT, region: US.region }, description: 'The airband relay in the United States: one small server and DynamoDB' });
new AirbandRelay(app, 'AirbandRelay', { env: { account: ACCOUNT, region: REGION }, description: 'The airband relay: one small task behind an existing load balancer' });
new AirbandSite(app, 'AirbandSite', { env: { account: ACCOUNT, region: 'us-east-1' }, description: 'airband.dev: the homepage, one static page behind a CDN' });
