import * as cloudflare from "@pulumi/cloudflare";
import * as fn0 from "@pulumi/fn0";
import * as pulumi from "@pulumi/pulumi";

const config = new pulumi.Config();

const accountId = config.require("CLOUDFLARE_ACCOUNT_ID");
const zoneId = config.require("CLOUDFLARE_ZONE_ID");
const domain = config.require("DOMAIN");
const awsWatchdogRegion = config.require("AWS_WATCHDOG_REGION");

const zone = cloudflare.getZone({
  zoneId,
});

const apiTokenPermissionGroups =
  cloudflare.getAccountApiTokenPermissionGroupsList({
    accountId,
    name: "DNS Write",
  });

const awsWatchdogVpc = new fn0.AwsWatchdogVpc("awsWatchdogVpc", {
  region: awsWatchdogRegion,
});

const cloudflareApiToken = new cloudflare.AccountToken("cloudflareApiToken", {
  accountId,
  name: "fn0Cloud",
  policies: [
    {
      effect: "allow",
      resources: {
        [`com.cloudflare.api.account.zone.${zoneId}`]: "*",
      },
      permissionGroups: apiTokenPermissionGroups.then((x) =>
        x.results.map((x) => ({ id: x.id }))
      ),
    },
  ],
  condition: {
    requestIp: {
      ins: [awsWatchdogVpc.ipv6CidrBlock],
    },
  },
});

const ociComputeWorker = new fn0.OciComputeWorker("ociComputeWorker", {
  region: config.require("OCI_COMPUTE_WORKER_REGION"),
  watchdogIpv6CidrBlock: awsWatchdogVpc.ipv6CidrBlock,
});

const awsWatchdog = new fn0.AwsWatchdog("awsWatchdog", {
  domain,
  region: awsWatchdogRegion,
  subnetId: awsWatchdogVpc.subnetId,
  securityGroupId: awsWatchdogVpc.securityGroupId,
  maxGracefulShutdownWaitSecs: 300,
  maxHealthyCheckRetries: 5,
  maxStartTimeoutSecs: 180,
  maxStartingCount: 1,
  ociWorkerInfraEnvs: ociComputeWorker.infraEnvs,
  cloudflareEnvs: {
    CLOUDFLARE_API_TOKEN: cloudflareApiToken.value,
    CLOUDFLARE_ASTERISK_DOMAIN: `*.${domain}`,
    CLOUDFLARE_ZONE_ID: zone.then((x) => x.id),
  },
});
