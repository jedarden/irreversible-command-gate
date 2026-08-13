# Prior art

Gathered across two research sessions investigating how other agent
operators solve "let the agent use real credentials/commands without being
able to cause irreversible damage or leak secrets." See the parent
conversation's OpenBao secret-rotation research for the broader survey
(proxy-injection tools, dynamic-credential/workload-identity tools,
capability tokens) — this file covers specifically the **command/verb-level
interception** category, which is this project's actual model, plus the
closest adjacent tools.

## Dicklesworthstone/destructive_command_guard

<https://github.com/Dicklesworthstone/destructive_command_guard>

Rust CLI interceptor with 49+ "packs," one per vendor tool (HashiCorp Vault,
AWS, GCP, Azure, Kubernetes, Terraform, PostgreSQL/MySQL/Redis/MongoDB,
payment processors, 1Password, CI/CD systems, CDNs, and more). Each pack
declares safe patterns (explicitly allowed, e.g. `vault kv get`, `vault
status`) and destructive patterns (blocked or flagged, e.g. `vault secrets
disable`, `vault kv destroy`, `vault policy delete`) via regex, each with a
severity level and a human-readable explanation of *why* it's dangerous and
a safer alternative. This is the direct model for this project's Vault
coverage gap.

Its author (Jeffrey Emanuel, `@doodlestein` on X) gives his own coding
agents standing, unproxied access to his personal HashiCorp Vault instance
— confirmed directly by him in reply to a question ("does this mean your
agent can also write new creds into hashi-vault?" → "Yes") — and this tool
is the reason he's comfortable with that: not narrow data access, but a
command-level blocklist on the irreversible verbs regardless of what the
agent otherwise does with its Vault access.

Architecturally: per-vendor Rust modules (packs), compiled into one binary
— modular by file, but not runtime-configurable in the sense of an
external data file. See `docs/notes/runtime-config-vs-hardcoded.md` for why
that distinction (modular-but-compiled vs. genuinely data-driven) matters
less than *where the rule source lives relative to what the guarded agent
can write to*.

## JeongJaeSoon/agent-guard

<https://github.com/JeongJaeSoon/agent-guard>

The inverse of `destructive_command_guard`: blocks the *read* side rather
than the destructive-write side. PreToolUse/PostToolUse hooks for Claude
Code and Codex that intercept secret-disclosure commands — `vault kv get`,
`aws secretsmanager get-secret-value`, `op read`, `printenv`, `cat .env` —
plus output-masking on generic read commands (`cat`, `head`) so a secret
that does get read doesn't surface in the transcript. Directly relevant
because it's built on the *same hook mechanism* (`PreToolUse`) this
environment already uses for `org-rule-guard.py` — closest available
reference for how a second, complementary hook might be structured
alongside it, or folded into the same one.

## hashicorp/vault-mcp-server

<https://github.com/hashicorp/vault-mcp-server>

Official HashiCorp MCP server. Holds the Vault token server-side and
exposes scoped MCP tools (mount management, KV read/write, PKI issuance)
instead of handing the agent a raw token — a proxy-injection design, not a
command-guard one. Documented limitation worth carrying forward: token
*custody* is solved (the agent never holds the Vault token itself), but
secret *value confidentiality* isn't automatic — depending on which tool is
invoked, the value can still land in the response back to the model.
HashiCorp's own docs scope it to "trusted networks, trusted clients,"
i.e. not positioned as a defense against an adversarial agent. Relevant as
a complementary layer (proxy for custody) rather than a substitute for
command-level guarding (this project's actual scope).

## askalf/redstamp

Deterministic, offline command firewall — no model in the decision loop,
claims 100%/100% on a 291-sample benchmark corpus. Blocks exfiltration
attempts and destructive commands generically (not vendor-CLI-aware the way
`destructive_command_guard`'s packs are). Ships as either a Claude Code
hook or an MCP proxy. Worth checking if this project's engine design wants
a second reference point for the "deterministic, no model involved"
constraint that `org-rule-guard.py` also follows.

## Sources

- [Dicklesworthstone/destructive_command_guard](https://github.com/Dicklesworthstone/destructive_command_guard)
- [JeongJaeSoon/agent-guard](https://github.com/JeongJaeSoon/agent-guard)
- [hashicorp/vault-mcp-server](https://github.com/hashicorp/vault-mcp-server)
- [@doodlestein on X](https://x.com/doodlestein) — confirmed standing agent
  Vault access, read and write, in a 2026-08-02 thread
