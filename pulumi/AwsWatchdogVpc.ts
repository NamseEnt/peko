import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";

export interface AwsWatchdogVpcArgs {
  region: pulumi.Input<string>;
}

export class AwsWatchdogVpc extends pulumi.ComponentResource {
  public readonly vpcId: pulumi.Output<string>;
  public readonly subnetId: pulumi.Output<string>;
  public readonly securityGroupId: pulumi.Output<string>;
  public readonly ipv6CidrBlock: pulumi.Output<string>;

  constructor(
    name: string,
    args: AwsWatchdogVpcArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:aws-watchdog-vpc", name, args, opts);

    const { region } = args;

    const vpc = new aws.ec2.Vpc("ipv6-vpc", {
      region,
      cidrBlock: "10.0.0.0/16",
      assignGeneratedIpv6CidrBlock: true,
      enableDnsHostnames: true,
    });
    this.vpcId = vpc.id;
    this.ipv6CidrBlock = vpc.ipv6CidrBlock;

    const eigw = new aws.ec2.EgressOnlyInternetGateway("ipv6-eigw", {
      region,
      vpcId: vpc.id,
    });

    const subnet = new aws.ec2.Subnet(
      "ipv6-native-subnet",
      {
        region,
        vpcId: vpc.id,
        ipv6CidrBlock: vpc.ipv6CidrBlock.apply((cidr) =>
          cidr.replace("/56", "/64")
        ),
        cidrBlock: "10.0.0.0/24",
        assignIpv6AddressOnCreation: true,
        enableResourceNameDnsAaaaRecordOnLaunch: true,
      },
      {
        deleteBeforeReplace: true,
      }
    );
    this.subnetId = subnet.id;

    const routeTable = new aws.ec2.RouteTable("ipv6-rt", {
      region,
      vpcId: vpc.id,
    });

    new aws.ec2.Route("ipv6-route", {
      region,
      routeTableId: routeTable.id,
      destinationIpv6CidrBlock: "::/0",
      egressOnlyGatewayId: eigw.id,
    });

    new aws.ec2.RouteTableAssociation("rt-assoc", {
      region,
      subnetId: subnet.id,
      routeTableId: routeTable.id,
    });

    const securityGroup = new aws.ec2.SecurityGroup("ipv6-lambda-sg", {
      region,
      vpcId: vpc.id,
      description: "Allow IPv6 traffic only",
      egress: [
        {
          protocol: "-1",
          fromPort: 0,
          toPort: 0,
          ipv6CidrBlocks: ["::/0"],
        },
      ],
    });
    this.securityGroupId = securityGroup.id;
  }
}
