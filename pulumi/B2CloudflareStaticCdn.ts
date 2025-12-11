import * as pulumi from "@pulumi/pulumi";
import * as b2 from "@pulumi/b2";
import * as random from "@pulumi/random";
import * as aws from "@pulumi/aws";
import * as cloudflare from "@pulumi/cloudflare";

export interface B2CloudflareStaticCdnArgs {
  zoneId: pulumi.Input<string>;
}

export class B2CloudflareStaticCdn extends pulumi.ComponentResource {
  constructor(
    name: string,
    args: B2CloudflareStaticCdnArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:b2-storage", name, args, opts);
    const b2Config = new pulumi.Config("b2");
    const b2KeyId = b2Config.requireSecret("applicationKeyId");
    const b2AppKey = b2Config.requireSecret("applicationKey");

    const bucketName = pulumi.interpolate`fn0-${
      new random.RandomString("b2-bucket-name", {
        length: 32,
        special: false,
        upper: false,
      }).result
    }`;

    new b2.Bucket("b2-bucket", {
      bucketName,
      bucketType: "allPublic",
      defaultServerSideEncryption: {
        algorithm: "AES256",
      },
    });

    const accountInfo = b2.getAccountInfo({});

    const s3Endpoint = accountInfo.then((info) => info.s3ApiUrl);

    const logBucketName = pulumi.interpolate`log-${bucketName}`;

    new b2.Bucket("b2-log-bucket", {
      bucketName: logBucketName,
      bucketType: "allPrivate",
    });

    const b2AwsProvider = new aws.Provider("b2-aws-provider", {
      accessKey: b2KeyId,
      secretKey: b2AppKey,
      skipCredentialsValidation: true,
      skipRequestingAccountId: true,
      skipMetadataApiCheck: true,
      s3UsePathStyle: true,
      endpoints: [
        {
          s3: s3Endpoint,
        },
      ],
    });

    new aws.s3.BucketLogging(
      "b2-bucket-logging",
      {
        bucket: bucketName,
        targetBucket: logBucketName,
        targetPrefix: "log/",
      },
      {
        provider: b2AwsProvider,
      }
    );

    const zone = cloudflare.Zone.get("zone", args.zoneId);

    new cloudflare.Ruleset(
      "cache-ruleset",
      {
        zoneId: zone.id,
        name: "cache-ruleset",
        kind: "zone",
        phase: "http_request_cache_settings",
        rules: [
          {
            action: "set_cache_settings",
            actionParameters: {
              cache: true,
              edgeTtl: {
                mode: "override_origin",
                default: 60 * 60 * 24 * 30,
                statusCodeTtls: [
                  { statusCode: 200, value: 60 * 60 * 24 * 30 },
                  { statusCode: 404, value: 30 },
                ],
              },
            },
            expression: pulumi.interpolate`http.host eq "cdn.${zone.name}"`,
          },
        ],
      },
      {
        deleteBeforeReplace: true,
      }
    );

    new cloudflare.Ruleset("b2-rewrite-rule", {
      zoneId: zone.id,
      name: "b2-rewrite-rule",
      kind: "zone",
      phase: "http_request_transform",
      rules: [
        {
          action: "rewrite",
          actionParameters: {
            uri: {
              path: {
                expression: pulumi.interpolate`concat("/file/${bucketName}", http.request.uri.path)`,
              },
            },
          },
          expression: pulumi.interpolate`http.host eq "cdn.${zone.name}" and not starts_with(http.request.uri.path, "/file/")`,
        },
      ],
    });

    new cloudflare.DnsRecord("cdn-dns-record", {
      zoneId: zone.id,
      name: "cdn",
      type: "CNAME",
      content: accountInfo.then((info) =>
        info.downloadUrl.replace("https://", "")
      ),
      ttl: 1,
      proxied: true,
    });
  }
}
