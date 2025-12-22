import * as pulumi from "@pulumi/pulumi";
import * as k8s from "@pulumi/kubernetes";
import * as docker from "@pulumi/docker";
import type { HqArgs } from "../hqArgs.schema";

export function deployHqApplication(
  parent: pulumi.Resource,
  {
    k8sProvider,
    hqImage,
    otlpEndpoint,
    hqArgs,
  }: {
    k8sProvider: k8s.Provider;
    hqImage: docker.Image;
    otlpEndpoint: pulumi.Output<string>;
    hqArgs: HqArgs;
  }
): {
  deployment: k8s.apps.v1.Deployment;
} {
  const appLabels = { app: "hq" };

  const hqArgsSecret = new k8s.core.v1.Secret(
    "hq-args-secret",
    {
      metadata: { labels: appLabels },
      stringData: {
        "hq-args.json": pulumi.jsonStringify(hqArgs),
      },
    },
    { provider: k8sProvider, parent }
  );

  const configMountPath = "/etc/hq-config";
  const configFilePath = `${configMountPath}/hq-args.json`;

  const deployment = new k8s.apps.v1.Deployment(
    "hq-deployment",
    {
      metadata: { labels: appLabels },
      spec: {
        replicas: 1,
        selector: { matchLabels: appLabels },
        template: {
          metadata: {
            labels: appLabels,
          },
          spec: {
            containers: [
              {
                name: appLabels.app,
                image: hqImage.repoDigest,
                ports: [{ containerPort: 8080 }],
                livenessProbe: {
                  httpGet: {
                    path: "/health",
                    port: 8080,
                  },
                  initialDelaySeconds: 15,
                  periodSeconds: 2,
                  timeoutSeconds: 2,
                  failureThreshold: 3,
                },
                volumeMounts: [
                  {
                    name: "hq-args-vol",
                    mountPath: configMountPath,
                    readOnly: true,
                  },
                ],
                env: [
                  {
                    name: "OTLP_ENDPOINT",
                    value: otlpEndpoint,
                  },
                  {
                    name: "HQ_ARGS_PATH",
                    value: configFilePath,
                  },
                ],
              },
            ],
            volumes: [
              {
                name: "hq-args-vol",
                secret: {
                  secretName: hqArgsSecret.metadata.name,
                },
              },
            ],
          },
        },
      },
    },
    { provider: k8sProvider, parent }
  );

  return { deployment };
}
