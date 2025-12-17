import * as pulumi from "@pulumi/pulumi";
import * as oci from "@pulumi/oci";
import * as command from "@pulumi/command";

export function createNetworking(
  parent: pulumi.Resource,
  {
    compartmentId,
    vcnId,
  }: {
    compartmentId: pulumi.Input<string>;
    vcnId: pulumi.Input<string>;
  }
): {
  regionalSubnet: oci.core.Subnet;
} {
  const internetGateway = new oci.core.InternetGateway(
    "igw",
    {
      compartmentId,
      vcnId,
    },
    { parent }
  );

  const routeTable = new oci.core.RouteTable(
    "route-table",
    {
      compartmentId,
      vcnId,
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
    },
    { parent }
  );

  const myIp = new command.local.Command(
    "my-ip",
    {
      create: "curl -s ifconfig.co",
      triggers: [new Date().toISOString()],
    },
    { parent }
  ).stdout;

  const securityList = new oci.core.SecurityList(
    "security-list",
    {
      compartmentId,
      vcnId,
      egressSecurityRules: [
        {
          destination: "0.0.0.0/0",
          destinationType: "CIDR_BLOCK",
          protocol: "all",
          stateless: false,
        },
        {
          destination: "::/0",
          destinationType: "CIDR_BLOCK",
          protocol: "all",
          stateless: false,
        },
      ],
      ingressSecurityRules: [
        {
          source: myIp.apply((ip) => `${ip}/32`),
          protocol: "all",
          stateless: false,
        },
        {
          source: "10.0.0.0/16",
          protocol: "all",
          stateless: false,
        },
      ],
    },
    { parent }
  );

  myIp.apply((x) => pulumi.log.info(`ip: ${x}`));

  const regionalSubnet = new oci.core.Subnet(
    "regional-subnet",
    {
      displayName: "fn0-hq-regional-subnet",
      compartmentId,
      vcnId,
      ipv4cidrBlocks: ["10.0.2.0/24"],
      routeTableId: routeTable.id,
      securityListIds: [securityList.id],
    },
    { parent, deleteBeforeReplace: true }
  );

  return { regionalSubnet };
}
