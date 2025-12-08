import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";
import { OciWorkerInfraEnvs } from "./OciComputeWorker";

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
  constructor(
    name: string,
    args: AwsWatchdogArgs,
    opts: pulumi.ComponentResourceOptions
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

    const lambdaFunction = new aws.lambda.CallbackFunction("watchdog", {
      region,
      timeout: 10,
      vpcConfig: {
        subnetIds: [subnetId],
        securityGroupIds: [securityGroupId],
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
                    Resource: healthRecordBucket.arn,
                  },
                ],
              } satisfies aws.iam.PolicyDocument)
              .apply((policyDoc) => JSON.stringify(policyDoc)),
          },
        ],
        managedPolicyArns: [
          aws.iam.ManagedPolicy.AWSLambdaVPCAccessExecutionRole,
          aws.iam.ManagedPolicy.AWSLambdaBasicExecutionRole,
        ],
      }),
      environment: {
        variables: pulumi
          .all([ociWorkerInfraEnvs, cloudflareEnvs])
          .apply(([ociWorkerInfraEnvs, cloudflareEnvs]) => ({
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
            MAX_START_TIMEOUT_SECS: pulumi.jsonStringify(maxStartTimeoutSecs),
            MAX_STARTING_COUNT: pulumi.jsonStringify(maxStartingCount),
            HEALTH_RECORD_BUCKET_NAME: healthRecordBucket.bucket,
            LOCK_TABLE_NAME: lockDdb.name,
            ...ociWorkerInfraEnvs,
            ...cloudflareEnvs,
          })),
      },
      callback: async () => {},
    });

    new aws.cloudwatch.EventTarget("watchdog-target", {
      region,
      rule: eventRule.name,
      arn: lambdaFunction.arn,
    });
  }
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
