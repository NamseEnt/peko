import * as pulumi from "@pulumi/pulumi";
import * as oci from "@pulumi/oci";
import * as docker from "@pulumi/docker";

export function createDockerRegistry(
  parent: pulumi.Resource,
  {
    compartmentId,
    nameSuffix,
    region,
  }: {
    compartmentId: pulumi.Input<string>;
    nameSuffix: pulumi.Output<string>;
    region: pulumi.Input<string>;
  }
): {
  hqImage: docker.Image;
} {

  const repo = new oci.artifacts.ContainerRepository(
    "hq-repo",
    {
      compartmentId,
      displayName: pulumi.interpolate`hq-repo-${nameSuffix}`,
      isPublic: true,
    },
    { parent, retainOnDelete: false }
  );

  const user = new oci.identity.User(
    "hq-user",
    {
      name: pulumi.interpolate`hq-user-${nameSuffix}`,
      description: "User for HQ deployment",
    },
    { parent }
  );

  const dockerGroup = new oci.identity.Group(
    "hq-docker-pusher-group",
    {
      name: pulumi.interpolate`hq-docker-pushers-${nameSuffix}`,
      description: "Group allowed to push to OCIR",
    },
    { parent }
  );

  new oci.identity.UserGroupMembership(
    "hq-membership",
    {
      userId: user.id,
      groupId: dockerGroup.id,
    },
    { parent }
  );

  new oci.identity.Policy(
    "ocir-push-policy",
    {
      compartmentId,
      name: pulumi.interpolate`allow-docker-push-${nameSuffix}`,
      description: "Policy to allow docker pushers to manage repos",
      statements: [
        pulumi.interpolate`Allow group ${dockerGroup.name} to manage repos in compartment id ${compartmentId}`,
      ],
    },
    { dependsOn: [dockerGroup], parent }
  );

  const authToken = new oci.identity.AuthToken(
    "hq-auth-token",
    {
      userId: user.id,
      description: "AuthToken for HQ deployment",
    },
    { parent }
  );

  const registryUrl = pulumi.interpolate`ocir.${region}.oci.oraclecloud.com`;

  const hqImage = new docker.Image(
    "hq-image",
    {
      imageName: pulumi.interpolate`${registryUrl}/${repo.namespace}/${repo.displayName}:v1`,
      build: {
        context: "../hq",
        platform: "linux/arm64",
      },
      registry: {
        server: registryUrl,
        username: pulumi.interpolate`${repo.namespace}/${user.name}`,
        password: authToken.token,
      },
    },
    { parent }
  );

  return { hqImage };
}
