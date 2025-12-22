import * as cloudflare from "@pulumi/cloudflare";
import * as fn0 from "@pulumi/fn0";
import * as pulumi from "@pulumi/pulumi";

const config = new pulumi.Config();

const accountId = config.require("cloudflareAccountId");
const zoneId = config.require("cloudflareZoneId");
const domain = config.require("domain");

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

const docDb = new fn0.TursoDocDb("docDb", {
  organizationSlug: config.require("tursoOrganizationSlug"),
  location: config.require("tursoLocation"),
});

const ociHeadQuarterVcn = new fn0.OciHeadQuarterVcn("ociHeadQuarterVcn", {
  region: config.require("ociHeadQuarterRegion"),
});

const ociComputeWorker = new fn0.OciComputeWorker("ociComputeWorker", {
  region: config.require("ociComputeWorkerRegion"),
  hqIpv6CidrBlocks: ociHeadQuarterVcn.ipv6cidrBlocks,
});

const ociHeadQuarter = new fn0.OciHeadQuarter("ociHeadQuarter", {
  ociRegion: config.require("ociHeadQuarterRegion"),
  compartmentId: ociHeadQuarterVcn.compartmentId,
  vcnId: ociHeadQuarterVcn.vcnId,
  ipv6cidrBlocks: ociHeadQuarterVcn.ipv6cidrBlocks,
  grafanaSlug: config.require("grafanaSlug"),
  grafanaRegion: config.require("grafanaRegion"),
  docDbUrl: docDb.url,
  docDbToken: docDb.token,
  sites: [],
});

export const kubeconfig = pulumi.secret(ociHeadQuarter.kubeconfig);

// new fn0.B2CloudflareStaticCdn("b2CloudflareStaticCdn", {
//   zoneId,
// });
