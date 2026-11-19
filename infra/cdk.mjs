// relay.airband.dev: the Airband relay as one small Fargate task behind an existing load balancer, keeping its
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
import { App, CfnOutput, Duration, Stack } from 'aws-cdk-lib';
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
new AirbandRelay(app, 'AirbandRelay', { env: { account: ACCOUNT, region: REGION }, description: 'The airband relay: one small task behind an existing load balancer' });
new AirbandSite(app, 'AirbandSite', { env: { account: ACCOUNT, region: 'us-east-1' }, description: 'airband.dev: the homepage, one static page behind a CDN' });
