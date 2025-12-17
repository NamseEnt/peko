import * as pulumi from "@pulumi/pulumi";
import * as k8s from "@pulumi/kubernetes";
import * as docker from "@pulumi/docker";
import { OciWorkerInfraEnvs } from "../OciComputeWorker";

export function deployHqApplication(
  parent: pulumi.Resource,
  {
    k8sProvider,
    hqImage,
    ociWorkerInfraEnvs,
  }: {
    k8sProvider: k8s.Provider;
    hqImage: docker.Image;
    ociWorkerInfraEnvs: pulumi.Input<OciWorkerInfraEnvs>;
  }
): {
  deployment: k8s.apps.v1.Deployment;
} {
  const appLabels = { app: "hq" };

  const deployment = new k8s.apps.v1.Deployment(
    "hq-deployment",
    {
      metadata: { labels: appLabels },
      spec: {
        replicas: 1,
        selector: { matchLabels: appLabels },
        template: {
          metadata: { labels: appLabels },
          spec: {
            containers: [
              {
                name: appLabels.app,
                image: hqImage.imageName,
                ports: [{ containerPort: 8080 }],
                livenessProbe: {
                  httpGet: {
                    path: "/health",
                    port: 8080,
                  },
                  initialDelaySeconds: 15,
                  periodSeconds: 5,
                  timeoutSeconds: 5,
                  failureThreshold: 3,
                },
                env: pulumi
                  .all([ociWorkerInfraEnvs])
                  .apply(([ociWorkerInfraEnvs]) => [
                    ...Object.entries(ociWorkerInfraEnvs).map(
                      ([name, value]) => ({
                        name,
                        value,
                      })
                    ),
                    {
                      name: "OTLP_ENDPOINT",
                      value: "http://alloy-service.monitoring:4317",
                    },
                  ]),
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
