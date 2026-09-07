# `forte cloud init` Specification

Status: **Implemented**.

This document defines the Cloudflare setup contract for the Forte CLI. The
command remains a single operation: it resolves the requested zone, registers
the project, provisions its Cloudflare resources, creates the credentials Forte
needs, and prints the DNS record that must be added.

## Invocation

```sh
forte cloud init \
  --project . \
  --project-name my-app \
  --zone example.com
```

The setup secret can later be replaced or removed with:

```sh
forte cloud rotate --project .
forte cloud clear --project . --yes
```

On the first run, `forte cloud init` asks for the Cloudflare setup token through
a masked prompt. It is never a command-line argument, environment variable,
printed value, or project/local credential-store entry.

`--setup-token-from-clipboard` replaces the masked prompt with a clipboard
hand-off: the command polls the OS clipboard, and the token is created in the
Cloudflare dashboard — by the user, or by an AI agent driving the browser — and
put on the clipboard with the dashboard's own Copy control. The command reads it
from the clipboard, verifies it against Cloudflare, and overwrites the
clipboard. The token is still never an argument, environment variable, printed
value, or credential-store entry, and an agent that only clicks Copy never
handles the value. The mode needs a desktop session; with no reachable
clipboard (SSH, CI) it is an error.

The first run uses the token only during bootstrap of the per-account
`fn0-broker` Worker. The token is saved as `FN0_SETUP_TOKEN` in the user's Cloudflare
account-level Secrets Store with the `workers` scope, and the Worker receives
it through a Secrets Store binding. Later runs use the broker URL already in
`Forte.toml` and do not ask for the setup token again.

For a new project directory, the same non-secret account ID and broker URL are
also reused from Forte's user configuration. That file contains no Cloudflare
token.

## Arguments

| Argument | Required | Meaning |
|---|---|---|
| `-p, --project <dir>` | no | Forte project directory; defaults to `.` |
| `--project-name <name>` | for a new project | Project identity and DNS label |
| `--zone <name>` | for a new project | Cloudflare zone name, such as `example.com` |
| `--setup-token-from-clipboard` | no | Read the first-run setup token from the clipboard instead of a masked prompt |

`--domain` is not part of the default contract. The public hostname is
derived as `<project-name>.<zone>`.

For example, `my-app` in the `example.com` zone answers on
`my-app.example.com`.

## Project name validation

`--project-name` is one DNS hostname label, not an arbitrary display name. It
must satisfy all of these rules:

- 1 to 63 ASCII characters
- only lowercase `a-z`, digits, and `-`
- starts and ends with a lowercase letter or digit
- no spaces, dots, underscores, uppercase letters, or Unicode characters

The CLI rejects invalid input rather than normalizing it. The derived full
hostname must also fit within the DNS maximum length of 253 characters.

## Zone resolution

`--zone` receives the zone name, not the Cloudflare zone ID. `example.com` is
the zone name. Cloudflare's internal hexadecimal zone ID is resolved locally
for provisioning; `Forte.toml` stores the human-readable zone name.

The CLI must resolve the exact requested zone. It must not choose the first
zone returned by the API. A missing or inaccessible zone is an error.

## Configuration

After successful setup, `Forte.toml` stores:

- `project_id`
- `project_name`
- `zone`
- `domain`
- `cloudflare_account_id`
- `cloudflare_broker_url`

The `domain` value is derived from `project_name` and `zone`. On later runs,
the stored values are checked against the requested values. A mismatch is
reported as an error instead of starting a reconfiguration flow.

## Authentication and secrets

The setup token is a bootstrap credential with `User -> API Tokens -> Edit`.
The first run creates a short-lived token with the account permissions needed
to create or update the user's Secrets Store secret and deploy the broker
Worker, then revokes that temporary token. The setup token itself is sent only
to Cloudflare during bootstrap and is never sent to fn0-control or the broker
API as a request value.

The secret value the user supplies is not the one the broker keeps. Cloudflare
refuses to mint a token carrying `API Tokens` permissions through the API, so
the setup token cannot be re-created; during bootstrap the CLI rolls its secret
in place (`PUT /user/tokens/{id}/value`) and stores the rolled value as
`FN0_SETUP_TOKEN`. The token keeps its id, name, and policy; only the secret
string changes, and the string that passed through a prompt or the clipboard
stops working the moment the roll lands. `forte cloud rotate
--setup-token-from-clipboard` rolls the value the same way before handing it to
the broker.

The broker is the only component that reads `FN0_SETUP_TOKEN` after bootstrap.
It accepts fixed operations only: exact zone lookup, project resource
provisioning, WebSockets, origin certificate issuance, domain finalization,
cleanup of credentials from a rejected connection, and — for `forte destroy` —
project teardown (removing the project's DNS record, bucket custom domains,
origin certificate, and minted tokens, and optionally the emptied buckets).
The app DNS record is removed only when it is still the single proxied CNAME
pointing at fn0's origin; an owner-modified record is preserved and reported.
Every request forwards the Forte control credential to fn0-control, which
verifies login and project ownership before the broker uses the setup token.

The bootstrap credential is reusable across projects. Project credentials are
created with only the permissions and resource scope required by each project.

## Operation order

1. Validate all local arguments.
2. Load the saved broker configuration, or read the setup token (prompt or
   clipboard), roll its secret, and install the user's account-level broker
   Worker and Secrets Store binding around the rolled value.
3. Resolve the exact Cloudflare zone name through the broker.
4. Derive and validate `<project-name>.<zone>`.
5. Create or resolve the Forte project identity.
6. Write the cloud fields, account ID, and broker URL to `Forte.toml` so a
   failed retry reuses the same project identity and broker.
7. Provision or reuse the project's Cloudflare buckets and zone resources
   through the broker.
8. Create or reuse the project's narrow credentials.
9. Ask the broker to issue the origin certificate, register it with fn0, and
   finalize CORS, cache, and DNS settings.
10. Print the required proxied CNAME record.

Every step must be safe to retry. Existing resources belonging to the same
project are reused rather than duplicated.

## Output and failure behavior

Normal output is human-readable text. Errors go to stderr and use a non-zero
exit status. JSON output is not required by this contract.

The command must fail before making Cloudflare changes when local validation
fails, including:

- missing `--project-name` or `--zone` for a new project
- invalid DNS label
- inaccessible zone
- a configuration mismatch on an existing project

If the account has no saved broker configuration, an empty setup-token prompt
is also an error. Token rotation also uses a masked prompt and never reads a
local environment variable. Both `init` and `rotate` accept
`--setup-token-from-clipboard` instead; in that mode an unreachable clipboard,
or no accepted token within the poll window, is an error.
