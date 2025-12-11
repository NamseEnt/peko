import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";
import { OciWorkerInfraEnvs } from "./OciComputeWorker";
import { FileArchive } from "@pulumi/pulumi/asset";

export interface AwsWatchdogArgs {
  region: pulumi.Input<string>;
  subnetId: pulumi.Input<string>;
  securityGroupId: pulumi.Input<string>;
  maxGracefulShutdownWaitSecs: pulumi.Input<number>;
  maxHealthyCheckRetries: pulumi.Input<number>;
  maxStartTimeoutSecs: pulumi.Input<number>;
  maxStartingCount: pulumi.Input<number>;
  domain: pulumi.Input<string>;
  ociWorkerInfraEnvs: pulumi.Input<OciWorkerInfraEnvs>;
  cloudflareEnvs: pulumi.Input<CloudflareEnvs>;
}

export interface CloudflareEnvs {
  CLOUDFLARE_ZONE_ID: pulumi.Input<string>;
  CLOUDFLARE_ASTERISK_DOMAIN: pulumi.Input<string>;
  CLOUDFLARE_API_TOKEN: pulumi.Input<string>;
}

export class AwsWatchdog extends pulumi.ComponentResource {
  public readonly lambdaFunctionName: pulumi.Output<string>;

  constructor(
    name: string,
    args: AwsWatchdogArgs,
    opts?: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:aws-watchdog", name, args, opts);

    const {
      region,
      subnetId,
      securityGroupId,
      ociWorkerInfraEnvs,
      maxGracefulShutdownWaitSecs,
      maxHealthyCheckRetries,
      maxStartTimeoutSecs,
      maxStartingCount,
      domain,
      cloudflareEnvs,
    } = args;

    const eventRule = new aws.cloudwatch.EventRule("watchdog", {
      region,
      name: `watchdog-${name}`,
      scheduleExpression: "rate(1 minute)",
    });

    const lockDdb = setLockDdb(region);
    const healthRecordBucket = setHealthRecordBucket(region);
    const workerHealthChecker = setWorkerHealthChecker(
      region,
      subnetId,
      securityGroupId
    );

    const lambdaFunction = new aws.lambda.Function("watchdog", {
      region,
      timeout: 30,
      architectures: ["arm64"],
      code: new FileArchive("../watchdog/target/lambda/watchdog/bootstrap.zip"),
      handler: "bootstrap",
      packageType: "Zip",
      runtime: "provided.al2023",
      environment: {
        variables: pulumi
          .all([ociWorkerInfraEnvs, cloudflareEnvs])
          .apply(([ociWorkerInfraEnvs, cloudflareEnvs]) => ({
            LOCK_AT: "dynamodb",
            HEALTH_RECORDER_AT: "s3",
            WORKER_INFRA_AT: "oci",
            DNS_AT: "cloudflare",
            DOMAIN: domain,
            RUST_BACKTRACE: "1",
            MAX_GRACEFUL_SHUTDOWN_WAIT_SECS: pulumi.jsonStringify(
              maxGracefulShutdownWaitSecs
            ),
            MAX_HEALTHY_CHECK_RETRIES: pulumi.jsonStringify(
              maxHealthyCheckRetries
            ),
            MAX_START_TIMEOUT_SECS: pulumi.jsonStringify(maxStartTimeoutSecs),
            MAX_STARTING_COUNT: pulumi.jsonStringify(maxStartingCount),
            HEALTH_RECORD_BUCKET_NAME: healthRecordBucket.bucket,
            LOCK_TABLE_NAME: lockDdb.name,
            WORKER_HEALTH_CHECKER_FN_NAME: workerHealthChecker.lambdaFunction.name,
            ...ociWorkerInfraEnvs,
            ...cloudflareEnvs,
          })),
      },
      role: new aws.iam.Role("watchdog-role", {
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
            name: "watchdog-policy",
            policy: pulumi
              .output({
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
                    Resource: healthRecordBucket.arn.apply((arn) => `${arn}/*`),
                  },
                  {
                    Effect: "Allow",
                    Action: ["s3:ListBucket"],
                    Resource: healthRecordBucket.arn,
                  },
                  {
                    Effect: "Allow",
                    Action: ["lambda:InvokeFunction"],
                    Resource: workerHealthChecker.lambdaFunction.arn,
                  },
                ],
              } satisfies aws.iam.PolicyDocument)
              .apply((policyDoc) => JSON.stringify(policyDoc)),
          },
        ],
        managedPolicyArns: [
          aws.iam.ManagedPolicy.AWSLambdaBasicExecutionRole,
        ],
      }).arn,
    });

    // new aws.lambda.Permission("watchdog-permission", {
    //   action: "lambda:InvokeFunction",
    //   function: lambdaFunction.arn,
    //   principal: "events.amazonaws.com",
    //   sourceArn: eventRule.arn,
    // });

    // new aws.cloudwatch.EventTarget("watchdog-target", {
    //   region,
    //   rule: eventRule.name,
    //   arn: lambdaFunction.arn,
    // });

    this.lambdaFunctionName = lambdaFunction.name;
    this.registerOutputs({
      lambdaFunctionName: this.lambdaFunctionName,
    });
  }
}

function setWorkerHealthChecker(
  region: pulumi.Input<string>,
  subnetId: pulumi.Input<string>,
  securityGroupId: pulumi.Input<string>
) {
  const lambdaFunction = new aws.lambda.Function("worker-health-checker", {
    region,
    timeout: 30,
    architectures: ["arm64"],
    vpcConfig: {
      subnetIds: [subnetId],
      securityGroupIds: [securityGroupId],
    },
    code: new FileArchive(
      "../worker-health-checker/target/lambda/worker-health-checker/bootstrap.zip"
    ),
    handler: "bootstrap",
    packageType: "Zip",
    runtime: "provided.al2023",
    environment: {
      variables: {
        RUST_BACKTRACE: "1",
      },
    },
    role: new aws.iam.Role("worker-health-checker-role", {
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
      managedPolicyArns: [
        aws.iam.ManagedPolicy.AWSLambdaVPCAccessExecutionRole,
        aws.iam.ManagedPolicy.AWSLambdaBasicExecutionRole,
      ],
    }).arn,
  });

  return { lambdaFunction };
}

function setLockDdb(region: pulumi.Input<string>) {
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

function setHealthRecordBucket(region: pulumi.Input<string>) {
  const healthRecordBucket = new aws.s3.Bucket("health-record-bucket", {
    region,
  });

  return healthRecordBucket;
}
