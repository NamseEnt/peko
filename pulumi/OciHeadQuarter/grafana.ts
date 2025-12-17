import * as pulumi from "@pulumi/pulumi";
import * as k8s from "@pulumi/kubernetes";
import * as grafana from "@pulumiverse/grafana";

// Please make grafana stack manually first with slug.
export function hqGrafana(
  parent: pulumi.Resource,
  {
    slug,
    k8sProvider,
    suffix,
  }: {
    regionSlug: pulumi.Input<string>;
    slug: pulumi.Input<string>;
    k8sProvider: k8s.Provider;
    suffix: pulumi.Output<string>;
  }
) {
  const stack = pulumi.all([slug]).apply(([slug]) =>
    grafana.cloud.getStack({
      slug,
    })
  );

  const config = new pulumi.Config("grafana");
  const password = config.require("cloudAccessPolicyToken");
  const releaseName = pulumi.interpolate`grafana-k8s-monitoring-${suffix}`;

  new k8s.helm.v3.Release(
    "grafana-k8s-monitoring",
    {
      name: releaseName,
      chart: "k8s-monitoring",
      repositoryOpts: {
        repo: "https://grafana.github.io/helm-charts",
      },
      namespace: "monitoring",
      createNamespace: true,
      values: {
        cluster: {
          name: stack.clusterSlug,
        },
        destinations: [
          {
            name: "grafana-cloud-metrics",
            type: "prometheus",
            url: stack.prometheusUrl,
            auth: {
              type: "basic",
              username: stack.prometheusUserId,
              password,
            },
          },
          {
            name: "grafana-cloud-logs",
            type: "loki",
            url: stack.logsUrl,
            auth: {
              type: "basic",
              username: stack.logsUserId,
              password,
            },
          },
          {
            name: "gc-otlp-endpoint",
            type: "otlp",
            url: stack.otlpUrl,
            protocol: "http",
            auth: {
              type: "basic",
              username: stack.id,
              password,
            },
            metrics: {
              enabled: true,
            },
            logs: {
              enabled: true,
            },
            traces: {
              enabled: true,
            },
          },
        ],
        clusterMetrics: {
          enabled: true,
          opencost: {
            enabled: true,
            metricsSource: "grafana-cloud-metrics",
            opencost: {
              exporter: {
                defaultClusterId: stack.clusterSlug,
              },
              prometheus: {
                existingSecretName: pulumi.interpolate`grafana-cloud-metrics-${releaseName}`,
                external: {
                  url: stack.prometheusUrl,
                },
              },
            },
          },
          kepler: {
            enabled: true,
          },
        },
        clusterEvents: {
          enabled: true,
        },
        podLogs: {
          enabled: true,
        },
        applicationObservability: {
          enabled: true,
          receivers: {
            otlp: {
              grpc: {
                enabled: true,
                port: 4317,
              },
              http: {
                enabled: true,
                port: 4318,
              },
            },
            zipkin: {
              enabled: true,
              port: 9411,
            },
          },
        },
        "alloy-metrics": {
          enabled: true,
          alloy: {
            extraEnv: [
              {
                name: "GCLOUD_RW_API_KEY",
                valueFrom: {
                  secretKeyRef: {
                    name: "alloy-metrics-remote-cfg-grafana-k8s-monitoring",
                    key: "password",
                  },
                },
              },
              {
                name: "CLUSTER_NAME",
                value: stack.clusterSlug,
              },
              {
                name: "NAMESPACE",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.namespace",
                  },
                },
              },
              {
                name: "POD_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.name",
                  },
                },
              },
              {
                name: "GCLOUD_FM_COLLECTOR_ID",
                value:
                  "grafana-k8s-monitoring-$(CLUSTER_NAME)-$(NAMESPACE)-$(POD_NAME)",
              },
            ],
          },
          remoteConfig: {
            enabled: true,
            url: stack.fleetManagementUrl,
            auth: {
              type: "basic",
              username: stack.fleetManagementUserId,
              password,
            },
          },
        },
        "alloy-singleton": {
          enabled: true,
          alloy: {
            extraEnv: [
              {
                name: "GCLOUD_RW_API_KEY",
                valueFrom: {
                  secretKeyRef: {
                    name: "alloy-singleton-remote-cfg-grafana-k8s-monitoring",
                    key: "password",
                  },
                },
              },
              {
                name: "CLUSTER_NAME",
                value: stack.clusterSlug,
              },
              {
                name: "NAMESPACE",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.namespace",
                  },
                },
              },
              {
                name: "POD_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.name",
                  },
                },
              },
              {
                name: "GCLOUD_FM_COLLECTOR_ID",
                value:
                  "grafana-k8s-monitoring-$(CLUSTER_NAME)-$(NAMESPACE)-$(POD_NAME)",
              },
            ],
          },
          remoteConfig: {
            enabled: true,
            url: stack.fleetManagementUrl,
            auth: {
              type: "basic",
              username: stack.fleetManagementUserId,
              password,
            },
          },
        },
        "alloy-logs": {
          enabled: true,
          alloy: {
            extraEnv: [
              {
                name: "GCLOUD_RW_API_KEY",
                valueFrom: {
                  secretKeyRef: {
                    name: "alloy-logs-remote-cfg-grafana-k8s-monitoring",
                    key: "password",
                  },
                },
              },
              {
                name: "CLUSTER_NAME",
                value: stack.clusterSlug,
              },
              {
                name: "NAMESPACE",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.namespace",
                  },
                },
              },
              {
                name: "POD_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.name",
                  },
                },
              },
              {
                name: "NODE_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "spec.nodeName",
                  },
                },
              },
              {
                name: "GCLOUD_FM_COLLECTOR_ID",
                value:
                  "grafana-k8s-monitoring-$(CLUSTER_NAME)-$(NAMESPACE)-alloy-logs-$(NODE_NAME)",
              },
            ],
          },
          remoteConfig: {
            enabled: true,
            url: stack.fleetManagementUrl,
            auth: {
              type: "basic",
              username: stack.fleetManagementUserId,
              password,
            },
          },
        },
        "alloy-receiver": {
          enabled: true,
          alloy: {
            extraPorts: [
              {
                name: "otlp-grpc",
                port: 4317,
                targetPort: 4317,
                protocol: "TCP",
              },
              {
                name: "otlp-http",
                port: 4318,
                targetPort: 4318,
                protocol: "TCP",
              },
              {
                name: "zipkin",
                port: 9411,
                targetPort: 9411,
                protocol: "TCP",
              },
            ],
            extraEnv: [
              {
                name: "GCLOUD_RW_API_KEY",
                valueFrom: {
                  secretKeyRef: {
                    name: "alloy-receiver-remote-cfg-grafana-k8s-monitoring",
                    key: "password",
                  },
                },
              },
              {
                name: "CLUSTER_NAME",
                value: stack.clusterSlug,
              },
              {
                name: "NAMESPACE",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.namespace",
                  },
                },
              },
              {
                name: "POD_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "metadata.name",
                  },
                },
              },
              {
                name: "NODE_NAME",
                valueFrom: {
                  fieldRef: {
                    fieldPath: "spec.nodeName",
                  },
                },
              },
              {
                name: "GCLOUD_FM_COLLECTOR_ID",
                value:
                  "grafana-k8s-monitoring-$(CLUSTER_NAME)-$(NAMESPACE)-alloy-receiver-$(NODE_NAME)",
              },
            ],
          },
          remoteConfig: {
            enabled: true,
            url: stack.fleetManagementUrl,
            auth: {
              type: "basic",
              username: stack.fleetManagementUserId,
              password,
            },
          },
        },
      },
    },
    { provider: k8sProvider, parent }
  );
}
