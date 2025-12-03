import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";

export interface AwsWatchdogWakerArgs {
  region: pulumi.Input<aws.Region>;
  maxGracefulShutdownWaitSecs: pulumi.Input<number>;
  maxHealthyCheckRetries: pulumi.Input<number>;
  domain: pulumi.Input<string>;
  ociWorkerInfraEnvs: pulumi.Input<{
    OCI_PRIVATE_KEY_BASE64: pulumi.Input<string>;
    OCI_USER_ID: pulumi.Input<string>;
    OCI_FINGERPRINT: pulumi.Input<string>;
    OCI_TENANCY_ID: pulumi.Input<string>;
    OCI_REGION: pulumi.Input<string>;
    OCI_COMPARTMENT_ID: pulumi.Input<string>;
  }>;
}

export class AwsWatchdogWaker extends pulumi.ComponentResource {
  public readonly ipv6CiderBlock: pulumi.Output<string>;

  constructor(
    name: string,
    args: AwsWatchdogWakerArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:aws-watchdog-waker", name, args, opts);

    const {
      region,
      ociWorkerInfraEnvs,
      maxGracefulShutdownWaitSecs,
      maxHealthyCheckRetries,
      domain,
    } = args;

    const { vpc, subnet, securityGroup } = setVpc(region);

    const eventRule = new aws.cloudwatch.EventRule("watchdog-waker", {
      region,
      name: `watchdog-waker-${name}`,
      scheduleExpression: "rate(1 minute)",
    });

    const lockDdb = setLockDdb(region);
    const healthRecordBucket = setHealthRecordBucket(region);

    const lambdaFunction = new aws.lambda.CallbackFunction("watchdog-waker", {
      region,
      timeout: 10,
      vpcConfig: {
        subnetIds: [subnet.id],
        securityGroupIds: [securityGroup.id],
      },
      role: new aws.iam.Role("watchdog-waker-role", {
        assumeRolePolicy: {
          Version: "2012-10-17",
          Statement: [
            {
              Effect: "Allow",
              Principal: {
                Service: "lambda.amazonaws.com",
              },
              Action: "sts:AssumeRole",
            },
          ],
        },
        inlinePolicies: [
          {
            policy: JSON.stringify({
              Version: "2012-10-17",
              Statement: [
                {
                  Effect: "Allow",
                  Action: ["dynamodb:PutItem", "dynamodb:GetItem"],
                  Resource: lockDdb.arn,
                },
                {
                  Effect: "Allow",
                  Action: ["s3:PutObject", "s3:GetObject"],
                  Resource: healthRecordBucket.arn,
                },
              ],
            } satisfies aws.iam.PolicyDocument),
          },
        ],
        managedPolicyArns: [
          aws.iam.ManagedPolicy.AWSLambdaVPCAccessExecutionRole,
          aws.iam.ManagedPolicy.AWSLambdaBasicExecutionRole,
        ],
      }),
      environment: {
        variables: {
          LOCK_AT: "dynamodb",
          HEALTH_RECORDER_AT: "s3",
          WORKER_INFRA_AT: "oci",
          DOMAIN: domain,
          MAX_GRACEFUL_SHUTDOWN_WAIT_SECS: pulumi.jsonStringify(
            maxGracefulShutdownWaitSecs
          ),
          MAX_HEALTHY_CHECK_RETRIES: pulumi.jsonStringify(
            maxHealthyCheckRetries
          ),
          HEALTH_RECORD_BUCKET_NAME: healthRecordBucket.bucket,
          LOCK_TABLE_NAME: lockDdb.name,
          ...ociWorkerInfraEnvs,
        },
      },
      callback: async () => {},
    });

    new aws.cloudwatch.EventTarget("watchdog-waker-target", {
      region,
      rule: eventRule.name,
      arn: lambdaFunction.arn,
    });

    this.ipv6CiderBlock = vpc.ipv6CidrBlock;
  }
}

function setVpc(region: pulumi.Input<aws.Region>) {
  const vpc = new aws.ec2.Vpc("ipv6-vpc", {
    region,
    assignGeneratedIpv6CidrBlock: true,
    enableDnsHostnames: true,
  });

  const eigw = new aws.ec2.EgressOnlyInternetGateway("ipv6-eigw", {
    region,
    vpcId: vpc.id,
  });

  const subnet = new aws.ec2.Subnet("ipv6-native-subnet", {
    vpcId: vpc.id,
    ipv6CidrBlock: vpc.ipv6CidrBlock.apply((cidr) => {
      if (!cidr) return "";
      const prefix = cidr.split("::/")[0];
      return `${prefix}00::/64`;
    }),
    assignIpv6AddressOnCreation: true,
    ipv6Native: true,
    mapPublicIpOnLaunch: false,
  });

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
    description: "Allow outbound IPv6 traffic only",
    egress: [
      {
        protocol: "-1",
        fromPort: 0,
        toPort: 0,
        ipv6CidrBlocks: ["::/0"],
      },
    ],
  });

  return {
    vpc,
    eigw,
    subnet,
    routeTable,
    securityGroup,
  };
}

function setLockDdb(region: pulumi.Input<aws.Region>) {
  const lockDdb = new aws.dynamodb.Table("lock-ddb", {
    region,
    hashKey: "master_lock",
    attributes: [
      {
        name: "master_lock",
        type: "S",
      },
    ],
    writeCapacity: 1,
    readCapacity: 1,
  });

  return lockDdb;
}

function setHealthRecordBucket(region: pulumi.Input<aws.Region>) {
  const healthRecordBucket = new aws.s3.Bucket("health-record-bucket", {
    region,
  });

  return healthRecordBucket;
}
