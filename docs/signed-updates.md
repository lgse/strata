# Signed in-app updates

Strata authenticates in-place updates in process. Users need neither the GitHub
CLI nor a GitHub account, token, keyring, OpenSSL command, or signing tool.
Package-managed installations still use their package manager.

## Trust and format

`data/update-keys.json` is the trust store compiled into Strata. It contains raw
32-byte Ed25519 public keys encoded as lowercase hexadecimal. Private keys must
never enter the repository or a release artifact. A key ID is the lowercase
SHA-256 of the raw public key, not of its textual encoding.

Every release publishes these two files alongside its archives:

- `strata-update-manifest.json`
- `strata-update-manifest.signatures.json`

The manifest is UTF-8 JSON with exactly these fields:

```json
{
  "schema": 1,
  "repository": "lgse/strata",
  "tag": "v0.12.0",
  "source_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "artifacts": [
    {
      "name": "strata-0.12.0-x86_64-unknown-linux-gnu.tar.gz",
      "target": "x86_64-unknown-linux-gnu",
      "size": 123456,
      "sha256": "<64 lowercase hexadecimal characters>"
    }
  ]
}
```

The example values are illustrative, not a usable release. The publisher must
include both `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. The
consumer accepts at most eight unique, supported artifacts and verifies the
selected architecture and exact asset name. Files inside the archive may evolve;
the package directory, executable, and `SOURCE_COMMIT` remain required.

The signature envelope has `schema: 1` and a `signatures` array. Each entry has
only `key_id` (64 lowercase hex characters) and `signature` (128 hex characters).
The Ed25519 message is the bytes `strata-update-manifest-v1`, one NUL byte, then
the **exact published manifest bytes**, including its trailing newline. Readers
must not reserialize the JSON before checking the signature. At least one
signature from an embedded key must verify. Unknown keys do not confer trust;
multiple signatures allow old and new clients to coexist during rotation.

The client bounds manifest/signature downloads to 32 KiB/8 KiB respectively,
including bodies without Content-Length. It authenticates metadata before
requesting the archive, caps the archive at its signed size (at most 128 MiB),
and verifies its SHA-256 in Rust before extraction or execution. The signed tag,
repository, architecture, package name, and source commit must match the requested
release and package. No separately downloaded checksum or mutable API lookup is
an authentication authority. Stable releases sign the original build source SHA,
not the subsequent version-bump commit; previews sign their selected source SHA.

Malformed, unsigned, oversized, or mismatched updates are refused without
replacing the installed binary. GitHub's build-provenance attestations and
`.sha256` files remain published for independent verification and packaging, but
are not prerequisites or fallbacks for this client protocol.

## Release signing

The `sign` job in `.github/workflows/release.yml` runs after both build targets
succeed. It executes `scripts/sign_update_manifest.py` from `github.workflow_sha`,
**never from a user-selected prerelease checkout**. It does not unpack or execute
the downloaded binaries. The later publishing job requires this job to succeed
before tagging or publishing anything.

The `release-signing` GitHub environment must:

1. Restrict deployments to the default branch (`main` today).
2. Require approval from the release maintainer, who must review the selected
   source and version before approving.
3. Hold `UPDATE_SIGNING_KEY`, an Ed25519 private key in PEM format. During a
   planned rotation it may also hold `UPDATE_SIGNING_KEY_NEXT`.

Only the signing step receives those secrets. The script removes them from the
subprocess environment, writes mode-0600 key files in a private temporary
directory, verifies its own signatures, and removes the directory on normal
completion or a handled error. The hosted runner is disposable; a killed job
must not upload temporary key files. Only the two public manifest files are
uploaded by the signing job. Missing or untrusted keys stop publication.

Generate a new key on a trusted maintainer machine with a restrictive umask:

```bash
umask 077
openssl genpkey -algorithm ED25519 -out release-key.pem
openssl pkey -in release-key.pem -pubout -outform DER -out release-key-public.der
```

An Ed25519 SubjectPublicKeyInfo DER value is 44 bytes: the prefix
`302a300506032b6570032100`, followed by the 32-byte raw public key. Validate that
prefix and length before adding the raw public key's hex encoding to
`data/update-keys.json`. Upload the private PEM through standard input, not a
command argument:

```bash
gh secret set UPDATE_SIGNING_KEY --repo lgse/strata --env release-signing < release-key.pem
```

Keep an encrypted/offline backup under maintainer control. GitHub secrets cannot
be downloaded later. The CLI/OpenSSL commands above are **maintainer tooling**,
not application runtime dependencies. Never use the deterministic test key from
`scripts/test_sign_update_manifest.py` for a real release.

## Rotation and compromise

For a planned rotation:

1. Add the new public key alongside the old key in `data/update-keys.json` through
   a reviewed PR. Keep the old signing secret as `UPDATE_SIGNING_KEY`.
2. Set `UPDATE_SIGNING_KEY_NEXT` to the new private key. Publish a bridge release
   whose binary trusts both keys and whose manifest is signed by both keys.
3. Continue dual-signing while older clients must be able to upgrade directly.
   Removing the old signature strands clients that only trust the old key; there
   is no untrusted network-key override or automatic trust-on-first-use fallback.
4. Once that compatibility tradeoff is explicitly accepted, switch the primary
   secret to the new key, remove the secondary secret, and remove the old public
   key from subsequent binaries.

If a key is compromised, disable signing and publishing, revoke/remove the
secret, and distribute a trusted replacement through an independently verified
installation channel. An attacker holding an embedded trusted key can sign a
malicious update; an online revocation claim cannot repair that trust bootstrap.
Do not describe ordinary overlapping-key rotation as sufficient compromise
recovery for old clients.

## Rollout and limitations

Already published releases do not have these manifests. The new updater refuses
them and explains that the user must choose a newer signed release or install
manually. This includes an explicit return-to-stable request targeting an older
unsigned stable release. Publish the first signed stable release before expecting
that path to work; do not silently fall back to checksum-only or `gh` verification.
The first signed release can be installed through the existing manual/package
flow. No existing release is modified or re-signed automatically by this change.

This protocol authenticates release contents and identity, not the initial
installation or the freshness of GitHub's release feed. It does not prevent a
server from withholding an update. Normal update selection rejects older
versions; an explicit user-requested return to stable may intentionally select
an older signed version. There is no clock-dependent manifest expiry.

## Verification

`python3 -m unittest discover -s scripts -p 'test_*.py'` tests the publisher using
only deterministic, non-production keys. Rust tests consume the same committed
OpenSSL-generated vectors in `tests/fixtures/signed-update/`, cover tampering,
wrong identities, duplicate/oversized metadata, both architectures, source/hash
mismatches, and overlapping signatures, and ensure the fixture key is not in the
production trust store. The publisher also checks that its signing key is in that
same trust store before producing release metadata.
