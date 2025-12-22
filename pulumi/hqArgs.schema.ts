import * as pulumi from '@pulumi/pulumi';
export interface HqArgs {
  deploymentDb: pulumi.Input<DeploymentDbArgs>;
  sites: pulumi.Input<Array<SiteArgs>>;
}
export interface CloudflareDnsProviderArgs {
  apiToken: pulumi.Input<string>;
  asteriskDomain: pulumi.Input<string>;
  zoneId: pulumi.Input<string>;
}
export interface DeploymentDbArgs {
  token: pulumi.Input<string>;
  url: pulumi.Input<string>;
}
export interface DnsProviderArg {
  cloudflare?: pulumi.Input<CloudflareDnsProviderArgs>;
}
export interface HostProviderArg {
  ociContainerInstance?: pulumi.Input<OciContainerInstanceHostProviderArgs>;
}
export interface OciContainerInstanceHostProviderArgs {
  availabilityDomain: pulumi.Input<string>;
  compartmentId: pulumi.Input<string>;
  envs: pulumi.Input<Record<string, string>>;
  fingerprint: pulumi.Input<string>;
  image: pulumi.Input<string>;
  memoryInGbs: pulumi.Input<number>;
  ocpus: pulumi.Input<number>;
  privateKeyBase64: pulumi.Input<string>;
  region: pulumi.Input<string>;
  shape: pulumi.Input<string>;
  subnetId: pulumi.Input<string>;
  tenancyId: pulumi.Input<string>;
  userId: pulumi.Input<string>;
}
export interface SiteArgs {
  cert: pulumi.Input<string>;
  dnsProvider: pulumi.Input<DnsProviderArg>;
  hostProvider: pulumi.Input<HostProviderArg>;
}
