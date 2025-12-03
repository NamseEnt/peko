import * as pulumi from "@pulumi/pulumi";
import * as oci from "@pulumi/oci";
import * as tls from "@pulumi/tls";

export interface OciComputeWorkerArgs {
  region: pulumi.Input<region>;
}

export class OciComputeWorker extends pulumi.ComponentResource {
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

    const privateKey = new tls.PrivateKey("oci-api-key-pair", {
      algorithm: "RSA",
      rsaBits: 2048,
    });

    const oci_user = new oci.identity.User("watchdog-user", {
      description: "fn0 watchdog user",
    });

    const apiKey = new oci.identity.ApiKey("api-key", {
      userId: oci_user.id,
      keyValue: privateKey.publicKeyPem,
    });
  }
}

type region =
  | "ap-sydney-1"
  | "ap-melbourne-1"
  | "sa-saopaulo-1"
  | "sa-vinhedo-1"
  | "ca-montreal-1"
  | "ca-toronto-1"
  | "sa-santiago-1"
  | "sa-valparaiso-1"
  | "sa-bogota-1"
  | "eu-paris-1"
  | "eu-marseille-1"
  | "eu-frankfurt-1"
  | "ap-hyderabad-1"
  | "ap-mumbai-1"
  | "il-jerusalem-1"
  | "eu-milan-1"
  | "ap-osaka-1"
  | "ap-tokyo-1"
  | "mx-queretaro-1"
  | "mx-monterrey-1"
  | "eu-amsterdam-1"
  | "me-riyadh-1"
  | "me-jeddah-1"
  | "ap-singapore-1"
  | "ap-singapore-2"
  | "af-johannesburg-1"
  | "ap-seoul-1"
  | "ap-chuncheon-1"
  | "eu-madrid-1"
  | "eu-stockholm-1"
  | "eu-zurich-1"
  | "me-abudhabi-1"
  | "me-dubai-1"
  | "uk-london-1"
  | "uk-cardiff-1"
  | "us-ashburn-1"
  | "us-chicago-1"
  | "us-phoenix-1"
  | "us-saltlake-2"
  | "us-sanjose-1";
