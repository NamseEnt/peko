import * as pulumi from "@pulumi/pulumi";

export interface fn0Args {
  cdn: pulumi.Input<string>;
  location: pulumi.Input<OciLocation>;
}

export class fn0 extends pulumi.ComponentResource {
  constructor(
    name: string,
    args: fn0Args,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:fn0", name, args, opts);
  }
}
