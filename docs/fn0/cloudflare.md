# Bring your own Cloudflare account

Your project's object storage, public objects, deployed frontend assets,
CDN-cached pages and custom domain all run on **your** Cloudflare account, not
fn0's. You keep R2's free tier, your own purge budget and your own bill; fn0
runs the compute and holds no storage on your behalf.

This setup is required before a project can serve a frontend: fn0 Cloud has no
way to meter object storage usage on an account it does not own, so
bring-your-own-Cloudflare is the default premise for every project, not an
opt-in. There is no `fn0.dev` fallback to fall back to — every project gets a
hostname inside one of the owner's zones.

## What you need

A Cloudflare account with **a zone in it** — a domain whose nameservers point
at Cloudflare. Not just an account: an R2 custom domain has to live in a zone
in the same account as the bucket, and that hostname is where your frontend
assets get served from.

## One command

```sh
forte cloud init \
  --project . \
  --project-name my-app \
  --zone example.com
```

`forte login` must be run first — the command loads fn0 credentials to
register the project and fails immediately if they are absent.

The first time this runs for a Cloudflare account, it asks for the setup
token through a masked prompt — never a command-line argument, an
environment variable, or something printed or saved anywhere in the clear —
and uses it once to install a small broker Worker in your own account. After
that, the token lives only inside that Worker's Secrets Store; the CLI never
holds it again, and later runs for this project or any other project on the
same account reuse the broker without asking for the token a second time.
(`--setup-token-from-clipboard` swaps the masked prompt for a clipboard
hand-off, so an AI agent can create the token for you — see
[AI-assisted setup](#ai-assisted-setup).)

As part of initialization, the CLI enables WebSockets for the selected zone.
This is required for Forte WebSocket routes to complete their upgrade through
Cloudflare's proxy. It is safe to run repeatedly because the setting is
written to `on` each time.

`--zone` is the zone name, such as `example.com`. It is not the hexadecimal
zone ID shown in Cloudflare's API details. The CLI resolves the exact zone
named by the argument and never picks the first zone returned by the API.

`--project-name` is also a DNS label. It must contain only lowercase ASCII
letters, digits, and hyphens, be 1–63 characters long, and start and end with a
letter or digit. The public hostname is derived as:

```text
<project-name>.<zone>
```

For example, this project answers on `my-app.example.com`. A separate domain
argument is not needed.

## Setup credential

Setup has to create buckets, point a hostname at one, write one zone rule, add
one DNS record and sign a certificate. Create one reusable setup token.
Cloudflare dashboard → **My Profile → API Tokens →
Create Token → Create Custom Token**. Give it exactly one permission:

| Scope | Permission |
| --- | --- |
| User | API Tokens → Edit |

Type it in once, at the masked prompt the first `forte cloud init` for this
account shows. That first run uses it to install a broker Worker — a small
Cloudflare Worker that lives in your own account — and stores the token as a
secret in your account's own Secrets Store, bound only to that Worker. From
then on, the broker is the only thing that ever reads the token: it mints a
short-lived, narrowly-scoped provisioning token for each operation setup
needs, does the work, and revokes it. The CLI itself never holds the setup
token beyond that first prompt, and it is never sent to fn0.

This token is powerful: a token that can create tokens can create any token
allowed by the account. That's exactly why it never leaves your own
Cloudflare account after the first run. If it's ever compromised or you just
want a new one, `forte cloud rotate` replaces it in place; `forte cloud
clear` removes it without replacing it; `forte cloud destroy` removes the
whole broker, token included. See [`forte cloud`](../forte/cli.md#forte-cloud-init)
for all three.

## AI-assisted setup

`forte cloud init --setup-token-from-clipboard` and `forte cloud rotate
--setup-token-from-clipboard` take the setup token from the OS clipboard
instead of a masked prompt: the command waits, you (or an AI agent driving your
browser) create the token in the dashboard and click its **Copy** button, and
the command picks it up, verifies it against Cloudflare, and wipes the
clipboard. The token never becomes a command argument, an environment variable,
or a file, and — when an agent does the clicking — it need not pass through the
agent's own context: the agent only clicks the native Copy control.

During bootstrap `forte` rolls the token's secret in place — Cloudflare will
not re-create a token that can manage tokens, so the secret is regenerated
rather than the token cloned — and stores the rolled value. The token keeps its
name and permission; the string that briefly sat on the clipboard stops
working the moment the roll lands.

The dashboard steps are the same whichever tool does them:

1. `https://dash.cloudflare.com/profile/api-tokens` → **Create Token**.
2. Scroll to the **"Create Additional Tokens"** template → **Use template**.
   (Not "Create Custom Token" — `User -> API Tokens -> Edit` is not reliably
   offered there.)
3. The form is pre-filled with `User | API Tokens | Edit`. **Continue to
   summary** → **Create Token**.
4. Click **Copy**. `forte` takes it from there.

Signing in, two-factor auth, and any identity re-prompt are yours to do in the
browser — an agent must not type credentials. Creating the token is a real
change to your account, so approve it before the final click.

Claude Code users: run the `cloudflare-setup-token` skill, which drives this
in the browser.

## What the stored credentials can do

| Credential | What it can do |
| --- | --- |
| Worker R2 | read and write objects in this project's two object-storage buckets. Cannot reach the frontend-asset bucket, cannot reach another project's buckets, cannot delete a bucket, cannot call the Cloudflare API at all |
| Frontend-asset R2 | read and write objects in this project's frontend-asset bucket, and nothing else. Never sent to a worker |
| Purge | purge this one zone's cache. Nothing else — not DNS, not cache rules, not R2, not certificates |

Those limits are measured against the live API, not inferred from the
permission names.

So the worst a total compromise of fn0 can do to your Cloudflare account is
rewrite objects in the three buckets it created for this project, and clear your
cache.

## What gets created

Three buckets, all this project's alone. Nothing is shared with your other fn0
projects, so no key prefix is carrying the separation.

| Bucket | Holds | Reachable at |
| --- | --- | --- |
| `fn0-<project-id>-private-object-storage` | what `object_storage::private` writes | nowhere — signed requests only |
| `fn0-<project-id>-public-object-storage` | what `object_storage::public` writes | `fn0-<project-id>-public-object-storage.<zone>` |
| `fn0-<project-id>-frontend-asset` | your deployed frontend build | `fn0-<project-id>-frontend-asset.<zone>` |

The two public buckets answer on a hostname that is the bucket's own name in
your zone, so a bucket and the address it serves from cannot drift apart. Each
costs one DNS record.

fn0 adds one rule to your zone and leaves your own rules in place. It matches
`fn0-*-frontend-asset.<zone>` and
`fn0-*-public-object-storage.<zone>`; the cache rule also matches the
custom domains registered for fn0 projects. This covers every fn0 project you
add without a rule per project — a free zone allows ten of each, and a rule per
project would run out at ten projects. Both halves of each pattern are required
so a rule cannot swallow a hostname of your own.

The **cache rule** makes static HTML eligible for the CDN and respects the
origin's cache headers. Your other hostnames keep the zone setting.

Smart Tiered Cache is enabled for the zone so a cache miss in an edge location
can be filled by an upper tier instead of reaching the worker fleet directly.

The two buckets a browser can reach are also given a **CORS allowlist holding
one origin: your project's own domain**. Cloudflare keys a separate cache entry
per `Origin` value and `Origin` is not verified, so an allowlist of `*` would
let any site on the web read every one of those entries and bill the misses to
you. The allowlist moves with the domain, so changing the domain rewrites it.

Workers pick a connection up within about a second; no redeploy is needed.

**Set the project up before it stores anything.** A project has nowhere to
store and cannot serve a frontend until it is connected.

Connecting is first-time only, and there is no way back. Reconnecting a
project, rotating its Worker/frontend-asset/purge credentials, and moving it
to a different Cloudflare account are all unsupported — not merely
undocumented, but refused. If one of those three credentials is lost or
revoked, the project cannot be repaired through the CLI; treat them as things
you do not lose. (This is about the project's own credentials, not the setup
token — that one you can rotate. See [Setup credential](#setup-credential).)

## The domain

Not optional: a project answers on the hostname derived from its project name
and zone, and on nothing else. There is no `fn0.dev` fallback.

Signing an origin certificate needs a permission fn0 deliberately does not
hold. The CLI generates the key pair and the certificate request locally —
the private key never leaves your machine — and has the broker submit that
request to Cloudflare and sign it through your own Origin CA. The CLI then
uploads the certificate and key to fn0, and the broker writes the
**proxied** `CNAME` for that hostname into your zone, pointing at the fn0
origin hostname. Nothing is left for you to add by hand.

The record is written last, after fn0 holds the certificate and the zone
carries the cache rule, so the hostname resolves nowhere until everything
behind it is ready.

A hostname that is already taken is not silently overwritten. An existing
`CNAME` on that name is repointed — you named the hostname on the command line,
so where it resolves is what you are asking to change — but an `A` or `AAAA`
record there stops the command with an error instead. Changing a project's
domain removes the old record, and only that record: if the `CNAME` fn0 wrote
has since been edited, it is left in place and reported. A record still
pointing at fn0 is worth removing — the next project to register that hostname
inherits whatever reaches it.

The derived hostname is written to `Forte.toml`, and `forte deploy` refuses if
the stored project name, zone, and hostname disagree with the live project.
Changing the zone or project hostname requires a new project connection.

The record must stay orange-clouded. An Origin CA certificate is not valid for
a direct visitor connection, so switching the record to DNS-only breaks the
hostname immediately and visibly.

## What fn0 still holds

- The compute and the request routing
- Your document database (Turso), which is not part of this
- The bundle store your deployed code is distributed from. It holds compiled
  WebAssembly rather than anything your app stores, and it grows with deploys
  rather than with traffic, so it stays on fn0's account

## Removing a project

`forte destroy` empties fn0's buckets in your account but does not delete
them — fn0 holds an object-scoped credential there by design. The buckets are
yours to remove.

## Revoking

Deleting either R2 token in your Cloudflare dashboard breaks the project at
request time — there is no grace period, because every request signs against
it.

There is no recovery path for a project's own credentials. A project that is
already connected is refused a second connection, and nothing else can
replace a stored credential, so a revoked Worker, frontend-asset, or purge
token means the project has to be recreated.

The setup token is different: it isn't tied to one project, so losing or
revoking it doesn't touch anything already connected. `forte cloud rotate`
replaces it in the broker's Secrets Store; `forte cloud clear` removes it
without a replacement; `forte cloud destroy` removes the broker Worker and
its Secrets Store entirely, for every project that shares this Cloudflare
account.
