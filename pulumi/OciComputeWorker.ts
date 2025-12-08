import * as pulumi from "@pulumi/pulumi";
import * as oci from "@pulumi/oci";
import * as tls from "@pulumi/tls";

export interface OciComputeWorkerArgs {
  region: pulumi.Input<string>;
  watchdogIpv6CidrBlock: pulumi.Input<string>;
}

export interface OciWorkerInfraEnvs {
  OCI_PRIVATE_KEY_BASE64: pulumi.Output<string>;
  OCI_USER_ID: pulumi.Output<string>;
  OCI_FINGERPRINT: pulumi.Output<string>;
  OCI_TENANCY_ID: pulumi.Output<string>;
  OCI_REGION: pulumi.Output<string>;
  OCI_COMPARTMENT_ID: pulumi.Output<string>;
  OCI_INSTANCE_CONFIGURATION_ID: pulumi.Output<string>;
}

export class OciComputeWorker extends pulumi.ComponentResource {
  public readonly compartmentId: pulumi.Output<string>;
  public readonly instanceConfigurationId: pulumi.Output<string>;
  public readonly infraEnvs: pulumi.Output<OciWorkerInfraEnvs>;

  constructor(
    name: string,
    args: OciComputeWorkerArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:oci-compute-worker", name, args, opts);

    const compartment = new oci.identity.Compartment("compartment", {
      description: "Compartment for fn0 OCI Compute Worker",
      name: `fn0-${name}`,
      enableDelete: true,
    });

    this.compartmentId = compartment.id;

    const privateKey = new tls.PrivateKey("oci-api-key-pair", {
      algorithm: "RSA",
      rsaBits: 2048,
    });

    const oci_user = new oci.identity.User("watchdog-user", {
      description: "fn0 watchdog user",
    });

    const vcn = new oci.core.Vcn("vcn", {
      compartmentId: compartment.id,
      isIpv6enabled: true,
      isOracleGuaAllocationEnabled: true,
      cidrBlocks: ["10.0.0.0/16"],
    });

    const securityList = new oci.core.SecurityList("security-list", {
      compartmentId: compartment.id,
      vcnId: vcn.id,
      ingressSecurityRules: [
        ...cloudflareIpv4Ranges,
        ...clareflareIpv6Ranges,
        args.watchdogIpv6CidrBlock,
      ].map((source) => ({
        protocol: "6",
        source,
        tcpOptions: { min: 443, max: 443 },
      })),
      egressSecurityRules: [
        {
          destination: "0.0.0.0/0",
          protocol: "all",
        },
        {
          destination: "::/0",
          protocol: "all",
        },
      ],
    });

    const internetGateway = new oci.core.InternetGateway("igw", {
      compartmentId: compartment.id,
      vcnId: vcn.id,
    });

    const routeTable = new oci.core.RouteTable("route-table", {
      compartmentId: compartment.id,
      vcnId: vcn.id,
      routeRules: [
        {
          destination: "::/0",
          destinationType: "CIDR_BLOCK",
          networkEntityId: internetGateway.id,
        },
        {
          destination: "0.0.0.0/0",
          destinationType: "CIDR_BLOCK",
          networkEntityId: internetGateway.id,
        },
      ],
    });

    new oci.core.Subnet("subnet", {
      compartmentId: compartment.id,
      vcnId: vcn.id,
      ipv4cidrBlocks: ["10.0.0.0/24"],
      ipv6cidrBlocks: vcn.ipv6cidrBlocks.apply((x) =>
        x.map((x) => x.replace("/56", "/64"))
      ),
      prohibitInternetIngress: false,
      prohibitPublicIpOnVnic: false,
      securityListIds: [securityList.id],
      routeTableId: routeTable.id,
    });

    const imageId = compartment.id.apply((compartmentId) =>
      oci.core
        .getImages({
          compartmentId,
          operatingSystem: "Oracle Linux",
          operatingSystemVersion: "10",
          sortOrder: "DESC",
        })
        .then((x) => {
          const imageId = x.images.find(
            (x) => x.createImageAllowed && x.displayName.includes("-aarch64-")
          )?.id;

          if (!imageId) {
            throw new Error("can not find image");
          }

          return imageId;
        })
    );

    const instanceConfiguration = new oci.core.InstanceConfiguration(
      "instance-configuration",
      {
        compartmentId: compartment.id,
        instanceDetails: {
          instanceType: "compute",
          launchDetails: {
            shape: "VM.Standard.A1.Flex",
            shapeConfig: {
              ocpus: 1,
              memoryInGbs: 6,
            },
            sourceDetails: {
              sourceType: "image",
              imageId,
            },
          },
        },
      }
    );

    this.instanceConfigurationId = instanceConfiguration.id;

    this.infraEnvs = pulumi
      .all([privateKey, oci_user, compartment, instanceConfiguration])
      .apply(([privateKey, oci_user, compartment, instanceConfiguration]) => ({
        OCI_PRIVATE_KEY_BASE64: privateKey.privateKeyPem,
        OCI_USER_ID: oci_user.id,
        OCI_FINGERPRINT: privateKey.publicKeyFingerprintMd5,
        OCI_TENANCY_ID: oci_user.compartmentId,
        OCI_REGION: pulumi.output(args.region),
        OCI_COMPARTMENT_ID: compartment.id,
        OCI_INSTANCE_CONFIGURATION_ID: instanceConfiguration.id,
      }));
  }
}

export enum OciRegion {
  AP_SYDNEY_1 = "ap-sydney-1",
  AP_MELBOURNE_1 = "ap-melbourne-1",
  SA_SAOPAULO_1 = "sa-saopaulo-1",
  SA_VINHEDO_1 = "sa-vinhedo-1",
  CA_MONTREAL_1 = "ca-montreal-1",
  CA_TORONTO_1 = "ca-toronto-1",
  SA_SANTIAGO_1 = "sa-santiago-1",
  SA_VALPARAISO_1 = "sa-valparaiso-1",
  SA_BOGOTA_1 = "sa-bogota-1",
  EU_PARIS_1 = "eu-paris-1",
  EU_MARSEILLE_1 = "eu-marseille-1",
  EU_FRANKFURT_1 = "eu-frankfurt-1",
  AP_HYDERABAD_1 = "ap-hyderabad-1",
  AP_MUMBAI_1 = "ap-mumbai-1",
  IL_JERUSALEM_1 = "il-jerusalem-1",
  EU_MILAN_1 = "eu-milan-1",
  AP_OSAKA_1 = "ap-osaka-1",
  AP_TOKYO_1 = "ap-tokyo-1",
  MX_QUERETARO_1 = "mx-queretaro-1",
  MX_MONTERREY_1 = "mx-monterrey-1",
  EU_AMSTERDAM_1 = "eu-amsterdam-1",
  ME_RIYADH_1 = "me-riyadh-1",
  ME_JEDDAH_1 = "me-jeddah-1",
  AP_SINGAPORE_1 = "ap-singapore-1",
  AP_SINGAPORE_2 = "ap-singapore-2",
  AF_JOHANNESBURG_1 = "af-johannesburg-1",
  AP_SEOUL_1 = "ap-seoul-1",
  AP_CHUNCHEON_1 = "ap-chuncheon-1",
  EU_MADRID_1 = "eu-madrid-1",
  EU_STOCKHOLM_1 = "eu-stockholm-1",
  EU_ZURICH_1 = "eu-zurich-1",
  ME_ABU_DHABI_1 = "me-abudhabi-1",
  ME_DUBAI_1 = "me-dubai-1",
  UK_LONDON_1 = "uk-london-1",
  UK_CARDIFF_1 = "uk-cardiff-1",
  US_ASHBURN_1 = "us-ashburn-1",
  US_CHICAGO_1 = "us-chicago-1",
  US_PHOENIX_1 = "us-phoenix-1",
  US_SALT_LAKE_2 = "us-saltlake-2",
  US_SAN_JOSE_1 = "us-sanjose-1",
}

const cloudflareIpv4Ranges = [
  "173.245.48.0/20",
  "103.21.244.0/22",
  "103.22.200.0/22",
  "103.31.4.0/22",
  "141.101.64.0/18",
  "108.162.192.0/18",
  "190.93.240.0/20",
  "188.114.96.0/20",
  "197.234.240.0/22",
  "198.41.128.0/17",
  "162.158.0.0/15",
  "104.16.0.0/13",
  "104.24.0.0/14",
  "172.64.0.0/13",
  "131.0.72.0/22",
];

const clareflareIpv6Ranges = [
  "2400:cb00::/32",
  "2606:4700::/32",
  "2803:f800::/32",
  "2405:b500::/32",
  "2405:8100::/32",
  "2a06:98c0::/29",
  "2c0f:f248::/32",
];
