## Security policy

This project uses [Joy](https://github.com/joyint/joy) for product
management. Joy's identity layer (Auth) records a small set of fields
in `.joy/project.yaml` that look like credentials to keyword-based
secret scanners but are public by cryptographic design.

### Public fields, not secrets

| Field | What it is | Why it is public |
| --- | --- | --- |
| `verify_key` | Ed25519 public verification key per human member. The matching private key is derived transiently from the member's passphrase via Argon2id and never written to disk. | Used to verify signatures produced by that member. The public half is required in the repo for verification by anyone reading the project. |
| `kdf_nonce` | Public input to Argon2id key derivation, scoped per member. | Same role as a Bitwarden vault salt. Useless without the passphrase, which is never stored. |
| `enrollment_verifier` | Argon2id digest of a single-use onboarding password. | Cleared on first login. The plaintext OTP is shared out-of-band and never touches the repository. |
| `delegation_verifier` | Ed25519 public verification key bound to a per-(human, AI) delegation entry. | Verifies that delegation tokens were actually authorised by the named human. The matching private key lives off-repo at `~/.local/state/joy/delegations/<project>/<ai-member>.key` with mode 0600. |
| `attestation.signature` | Ed25519 signature by a manage-capable member over an attested member's email, capabilities, and `enrollment_verifier`. | Public proof of membership authorisation. |

No passphrase, no private key, and no decryptable secret is stored in
`.joy/project.yaml`. The threat model is documented in detail in the
[Auth document](https://github.com/joyint/joy/blob/main/docs/dev/vision/trustship/Auth.md)
in the Joy repository.

### Reporting a security issue

If you believe you have found a security issue in Joy itself, please
report it via [GitHub Security Advisories](https://github.com/joyint/joy/security/advisories/new)
in the Joy repository. Do not open public issues for security reports.

### Scanner false positives

If an automated scanner or SOC pipeline flags any of the fields listed
above, this document is the canonical explanation for why the match is
expected and not a leak. Please link to this file from the alert.
