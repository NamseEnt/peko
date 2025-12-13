import * as pulumi from "@pulumi/pulumi";
import * as aws from "@pulumi/aws";
import path = require("node:path");

export interface AwsLambdaCwasmCompilerArgs {
  region: pulumi.Input<string>;
  wasmBucket: pulumi.Input<string>;
  cWasmBucket: pulumi.Input<string>;
  queueArn: pulumi.Input<string>;
}

export class AwsLambdaCwasmCompiler extends pulumi.ComponentResource {
  constructor(
    name: string,
    args: AwsLambdaCwasmCompilerArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:aws-lambda-cwasm-compiler", name, args, opts);

    const { region, wasmBucket, queueArn, cWasmBucket } = args;

    const wasmtimeVersion = "39.0.1";

    const wasmtimeLayer = new aws.lambda.LayerVersion("wasmtime-layer", {
      region,
      layerName: wasmtimeVersion,
      code: new pulumi.asset.FileArchive(
        path.join(
          __dirname,
          `./wasmtime-v${wasmtimeVersion}-x86_64-linux.tar.xz`
        )
      ),
    });

    const zstdLayer = new aws.lambda.LayerVersion("zstd-layer", {
      region,
      layerName: "zstd-1.5.5-1",
      code: new pulumi.asset.FileArchive(path.join(__dirname, "./zstd")),
    });

    const wasmBucketArn = pulumi.interpolate`arn:aws:s3:::${wasmBucket}`;
    const cWasmBucketArn = pulumi.interpolate`arn:aws:s3:::${cWasmBucket}`;

    const lambda = new aws.lambda.CallbackFunction<aws.sqs.QueueEvent, void>(
      "cwasm-compiler-lambda",
      {
        region,
        layers: [wasmtimeLayer.arn, zstdLayer.arn],
        timeout: 20,
        memorySize: 10240,
        environment: {
          variables: {
            WASMTIME_VERSION: wasmtimeVersion,
            CWASM_BUCKET: cWasmBucket,
          },
        },
        callback: async (event) => {
          const {
            S3Client,
            GetObjectCommand,
            DeleteObjectCommand,
            PutObjectCommand,
          } = await import("@aws-sdk/client-s3");
          const { execSync } = await import("node:child_process");
          const { readFileSync, createWriteStream } = await import("node:fs");
          const { rm } = await import("node:fs/promises");

          const { pipeline } = await import("node:stream/promises");
          const path = await import("node:path");

          const s3Client = new S3Client({});

          const wasmtimeVer = process.env.WASMTIME_VERSION!;
          const cWasmBucketName = process.env.CWASM_BUCKET!;
          const wasmtimePath = `/opt/wasmtime-v${wasmtimeVer}-x86_64-linux/wasmtime`;
          const zstdPath = `/opt/zstd`;

          if (event.Records.length !== 1) {
            throw new Error(
              `Expected exactly one record, got ${event.Records.length}`
            );
          }
          const [record] = event.Records;
          const bucketRecord = JSON.parse(record.body) as aws.s3.BucketRecord;

          const bucket = bucketRecord.s3.bucket.name;
          const key = bucketRecord.s3.object.key;

          console.log(`Processing s3://${bucket}/${key}`);

          const wasmPath = path.join("/tmp", "input.wasm");
          const cwasmPath = path.join("/tmp", "output.cwasm");
          const cwasmZstdPath = path.join("/tmp", "output.cwasm.zst");

          console.log("clear up before start");
          await Promise.all([
            rm(wasmPath, { force: true }),
            rm(cwasmPath, { force: true }),
            rm(cwasmZstdPath, { force: true }),
          ]);

          console.log("get wasm from s3");
          const getCommand = new GetObjectCommand({
            Bucket: bucket,
            Key: key,
          });
          const response = await s3Client.send(getCommand);
          if (!response.Body) {
            console.log("no body");
            return;
          }
          await pipeline(
            response.Body.transformToWebStream(),
            createWriteStream(wasmPath)
          );

          console.log("compile wasm to cwasm");
          execSync(`${wasmtimePath} compile "${wasmPath}" -o "${cwasmPath}"`, {
            stdio: "inherit",
          });

          console.log("zstd cwasm");
          execSync(`${zstdPath} --ultra -22 "${cwasmPath}"`, {
            stdio: "inherit",
          });

          console.log("put cwasm to s3");
          const cwasmZstdBuffer = readFileSync(cwasmZstdPath);

          const cwasmKey = key.replace(/\.wasm$/, ".cwasm.zst");
          const putCommand = new PutObjectCommand({
            Bucket: cWasmBucketName,
            Key: cwasmKey,
            Body: cwasmZstdBuffer,
          });
          await Promise.all([
            await s3Client.send(putCommand),
            rm(wasmPath),
            rm(cwasmPath),
            rm(cwasmZstdPath),
          ]);

          console.log("delete wasm from s3");
          const deleteCommand = new DeleteObjectCommand({
            Bucket: bucket,
            Key: key,
          });
          await s3Client.send(deleteCommand);
        },
        role: new aws.iam.Role("lambda-role", {
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
              name: "s3-access-policy",
              policy: pulumi
                .all([wasmBucketArn, cWasmBucketArn])
                .apply(([wasmBucketArn, cWasmBucketArn]) =>
                  JSON.stringify({
                    Version: "2012-10-17",
                    Statement: [
                      {
                        Effect: "Allow",
                        Action: ["s3:GetObject", "s3:DeleteObject"],
                        Resource: `${wasmBucketArn}/*`,
                      },
                      {
                        Effect: "Allow",
                        Action: ["s3:ListBucket"],
                        Resource: wasmBucketArn,
                      },
                      {
                        Effect: "Allow",
                        Action: ["s3:PutObject"],
                        Resource: `${cWasmBucketArn}/*`,
                      },
                    ],
                  } satisfies aws.iam.PolicyDocument)
                ),
            },
          ],
          managedPolicyArns: [
            aws.iam.ManagedPolicy.AWSLambdaBasicExecutionRole,
            aws.iam.ManagedPolicy.AWSLambdaSQSQueueExecutionRole,
          ],
        }).arn,
      }
    );

    new aws.lambda.EventSourceMapping("sqs-trigger", {
      region,
      eventSourceArn: queueArn,
      functionName: lambda.name,
      batchSize: 1,
    });
  }
}
