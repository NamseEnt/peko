import * as cloudflare from "@pulumi/cloudflare";
import * as fn0 from "@pulumi/fn0";
import * as pulumi from "@pulumi/pulumi";

const config = new pulumi.Config();

const accountId = config.require("CLOUDFLARE_ACCOUNT_ID");
const zoneId = config.require("CLOUDFLARE_ZONE_ID");
const domain = config.require("DOMAIN");

const apiTokenPermissionGroups = Promise.all([
  cloudflare.getAccountApiTokenPermissionGroupsList({
    accountId,
    name: "DNS Write",
  }),
]).then((x) => x.flatMap((x) => x.results.map((x) => ({ id: x.id }))));

// const cloudflareApiToken = new cloudflare.AccountToken("cloudflareApiToken", {
//   accountId,
//   name: "fn0Cloud",
//   policies: [
//     {
//       effect: "allow",
//       resources: {
//         [`com.cloudflare.api.account.zone.${zoneId}`]: "*",
//       },
//       permissionGroups: apiTokenPermissionGroups,
//     },
//   ],
// });

const ociHeadQuarterVcn = new fn0.OciHeadQuarterVcn("ociHeadQuarterVcn", {
  region: config.require("OCI_HEAD_QUARTER_REGION"),
});

const ociComputeWorker = new fn0.OciComputeWorker("ociComputeWorker", {
  region: config.require("OCI_COMPUTE_WORKER_REGION"),
  hqIpv6CidrBlocks: ociHeadQuarterVcn.ipv6cidrBlocks,
});

const ociHeadQuarter = new fn0.OciHeadQuarter("ociHeadQuarter", {
  region: config.require("OCI_HEAD_QUARTER_REGION"),
  compartmentId: ociHeadQuarterVcn.compartmentId,
  vcnId: ociHeadQuarterVcn.vcnId,
  ipv6cidrBlocks: ociHeadQuarterVcn.ipv6cidrBlocks,
  ociWorkerInfraEnvs: ociComputeWorker.infraEnvs,
});

// const awsWatchdog = new fn0.AwsWatchdog("awsWatchdog", {
//   domain,
//   region: awsWatchdogRegion,
//   subnetId: awsWatchdogVpc.subnetId,
//   securityGroupId: awsWatchdogVpc.securityGroupId,
//   maxGracefulShutdownWaitSecs: 300,
//   maxHealthyCheckRetries: 5,
//   maxStartTimeoutSecs: 180,
//   maxStartingCount: 1,
//   ociWorkerInfraEnvs: ociComputeWorker.infraEnvs,
//   cloudflareEnvs: {
//     CLOUDFLARE_API_TOKEN: cloudflareApiToken.value,
//     CLOUDFLARE_ASTERISK_DOMAIN: `*.${domain}`,
//     CLOUDFLARE_ZONE_ID: zoneId,
//   },
// });

// new fn0.B2CloudflareStaticCdn("b2CloudflareStaticCdn", {
//   zoneId,
// });
