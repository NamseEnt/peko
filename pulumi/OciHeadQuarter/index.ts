import * as pulumi from "@pulumi/pulumi";
import * as random from "@pulumi/random";
import { hqGrafana } from "./grafana";
import { createNetworking } from "./networking";
import { createOkeCluster } from "./oke-cluster";
import { createDockerRegistry } from "./docker-registry";
import { deployK8sDashboard } from "./k8s-dashboard";
import { deployHqApplication } from "./hq-deployment";
import { SiteArgs } from "../hqArgs.schema";

export interface OciHeadQuarterArgs {
  ociRegion: pulumi.Input<string>;
  compartmentId: pulumi.Input<string>;
  vcnId: pulumi.Input<string>;
  ipv6cidrBlocks: pulumi.Input<string[]>;
  grafanaRegion: pulumi.Input<string>;
  grafanaSlug: pulumi.Input<string>;
  docDbUrl: pulumi.Input<string>;
  docDbToken: pulumi.Input<string>;
  sites: pulumi.Input<SiteArgs[]>;
}

export class OciHeadQuarter extends pulumi.ComponentResource {
  kubeconfig: pulumi.Output<string>;
  constructor(
    name: string,
    args: OciHeadQuarterArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:oci-head-quarter", name, args, opts);

    const { ociRegion, compartmentId, vcnId, docDbUrl, docDbToken, sites } =
      args;

    const nameSuffix8 = new random.RandomString(
      "name-suffix-8",
      {
        length: 8,
        special: false,
        upper: false,
      },
      { parent: this }
    ).result;

    const { regionalSubnet } = createNetworking(this, {
      compartmentId,
      vcnId,
      ipv6cidrBlocks: args.ipv6cidrBlocks,
    });

    const config = new pulumi.Config("oci");
    const tenancyOcid = config.require("tenancyOcid");
    const userOcid = config.require("userOcid");
    const fingerprint = config.require("fingerprint");
    const privateKey = config.require("privateKey");

    const { k8sProvider, kubeconfig } = createOkeCluster(this, {
      compartmentId,
      vcnId,
      regionalSubnetId: regionalSubnet.id,
      nameSuffix: nameSuffix8,
      region: ociRegion,
      tenancyOcid,
      userOcid,
      fingerprint,
      privateKey,
    });
    this.kubeconfig = kubeconfig;

    const { release: grafanaRelease, otlpEndpoint } = hqGrafana(this, {
      regionSlug: args.grafanaRegion,
      slug: args.grafanaSlug,
      k8sProvider: k8sProvider,
      suffix: nameSuffix8,
    });

    deployK8sDashboard(this, {
      k8sProvider,
      dependsOn: [grafanaRelease],
    });

    const { hqImage } = createDockerRegistry(this, {
      compartmentId,
      nameSuffix: nameSuffix8,
      region: ociRegion,
    });

    deployHqApplication(this, {
      k8sProvider,
      hqImage,
      otlpEndpoint,
      hqArgs: {
        sites,
        deploymentDb: {
          url: docDbUrl,
          token: docDbToken,
        },
      },
    });
  }
}
