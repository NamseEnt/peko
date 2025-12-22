import * as pulumi from "@pulumi/pulumi";
import { TursoGroup } from "./turso/group";
import { TursoDatabase } from "./turso/database";
import { TursoDatabaseToken } from "./turso/dbToken";
import { TursoTable } from "./turso/table";
import * as random from "@pulumi/random";

export interface TursoDocDbArgs {
  location: pulumi.Input<string>;
  organizationSlug: pulumi.Input<string>;
}

export class TursoDocDb extends pulumi.ComponentResource {
  public readonly url: pulumi.Output<string>;
  public readonly token: pulumi.Output<string>;

  constructor(
    name: string,
    args: TursoDocDbArgs,
    opts: pulumi.ComponentResourceOptions
  ) {
    super("pkg:index:turso-doc-db", name, args, opts);
    const { location, organizationSlug } = args;

    const nameSuffix8 = new random.RandomString(
      "name-suffix-8",
      {
        length: 8,
        special: false,
        upper: false,
      },
      { parent: this }
    ).result;

    const group = new TursoGroup(
      "group",
      {
        organizationSlug,
        name: "fn0-doc-db",
        location,
      },
      { parent: this }
    );

    const database = new TursoDatabase(
      "database",
      {
        organizationSlug,
        name: `fn0-doc-db-${nameSuffix8}`,
        group: group.name,
      },
      { parent: this }
    );

    const token = new TursoDatabaseToken(
      "token",
      {
        organizationSlug,
        databaseName: database.name,
      },
      { parent: this }
    );

    new TursoTable(
      "docs-table",
      {
        organizationSlug,
        jwt: token.jwt,
        databaseName: database.name,
        createTableSql: `
CREATE TABLE IF NOT EXISTS docs (
  pk BLOB NOT NULL,
  sk BLOB NOT NULL,
  value BLOB NOT NULL,
  PRIMARY KEY (pk, sk)
) WITHOUT ROWID;`.trim(),
      },
      { parent: this }
    );

    this.url = pulumi.interpolate`libsql://${database.name}.${location}.turso.io`;
    this.token = token.jwt;
  }
}
