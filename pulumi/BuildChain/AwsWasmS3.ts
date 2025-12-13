import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";

export interface AwsWasmS3Args {
  region: pulumi.Input<string>;
}

export class AwsWasmS3 extends pulumi.ComponentResource {
  public readonly bucket: pulumi.Output<string>;
  public readonly queueArn: pulumi.Output<string>;

  constructor(
    name: string,
    args: AwsWasmS3Args,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:s3-build-trigger-queue", name, args, opts);

    const { region } = args;

    const wasmBucket = new aws.s3.Bucket("wasm-bucket", {
      region,
    });
    this.bucket = wasmBucket.bucket;

    new aws.s3.BucketLifecycleConfiguration("wasm-bucket-lifecycle", {
      region,
      bucket: wasmBucket.bucket,
      rules: [
        {
          id: "wasm-bucket-lifecycle-rule",
          status: "Enabled",
          expiration: {
            days: 1,
          },
        },
      ],
    });

    const queue = new aws.sqs.Queue("queue", {
      region,
      fifoQueue: true,
    });
    this.queueArn = queue.arn;

    const queuePolicyDoc = aws.iam.getPolicyDocumentOutput({
      statements: [
        {
          effect: "Allow",
          principals: [
            {
              type: "Service",
              identifiers: ["s3.amazonaws.com"],
            },
          ],
          actions: ["sqs:SendMessage"],
          resources: [queue.arn],
          conditions: [
            {
              test: "ArnEquals",
              variable: "aws:SourceArn",
              values: [wasmBucket.arn],
            },
          ],
        },
      ],
    });

    const queuePolicy = new aws.sqs.QueuePolicy("allow-s3-send-message", {
      region,
      queueUrl: queue.id,
      policy: queuePolicyDoc.json,
    });

    new aws.s3.BucketNotification(
      "bucket-notification",
      {
        region,
        bucket: wasmBucket.bucket,
        queues: [
          {
            queueArn: queue.arn,
            events: ["s3:ObjectCreated:*"],
          },
        ],
      },
      { dependsOn: [queuePolicy] }
    );
  }
}
