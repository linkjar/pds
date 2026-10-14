# Unit 5, first part: accounts and legacy sessions

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §10.1](../SPEC.md#101-lifecycle)

Until this part the Candidate served accounts that the Reference had
created. It now creates its own and opens their sessions.

## 1. Result

| Method | State |
|---|---|
| `com.atproto.server.createAccount` | Built for a hosted handle. An account that brings its DID is refused, as without service auth at the pin. |
| `createSession`, `refreshSession`, `getSession`, `deleteSession` | Built. |
| `createAppPassword`, `listAppPasswords`, `revokeAppPassword` | Built. |

The writes run holds them to the Reference. `default/02-sessions` and the
new `writes/24-create-account` pass on both servers, and the comparison
recorded three new entries, all of them the duplicate app-password name,
a deliberate difference (SPEC §2.3): a second app password of one
name is 500 on the Reference and 400 on the Candidate.

## 2. How it is held to the Reference

- **Passwords.** A stored hash is `<salt>:<hash>`, scrypt with Node's
  defaults and 64 bytes, and the salt goes in as its text. The unit test
  holds the function to RFC 7914's third vector, which Node's
  `crypto.scrypt` also gives. The test accounts carry hashes that Node
  made, so a session opened by the Candidate checks a hash of the
  Reference's making. In the writes run the Candidate opens sessions with
  passwords that the Reference stored.
- **Tokens.** HS256 with `PDS_JWT_SECRET`, the claims and lifetimes of the
  pin: 120 minutes for an access token, 90 days for a refresh token, a
  grace period of 2 hours after a refresh, and one successor for a token
  that is used twice.
- **The identity.** The Candidate forms the `did:plc` genesis operation as
  `@did-plc/lib` does and signs it with the rotation key. In the writes
  run the stack's PLC directory, built from the pinned commit, accepted
  the operation and served the DID document.
- **The answers.** Nine refusals of `createAccount` and the refusals of
  the session methods are in the transcripts with status, error name and
  message, and none differs.

## 3. What is not built in this part

- An account that brings its DID and a PLC operation: migration, which
  needs service auth.
- The Reference's list of disposable email domains, and its check of an
  email address with `@hapi/address`. The Candidate checks the common
  shape of an address. An address that one accepts and the other refuses
  is possible; no scenario found one.
- The slur check on handles.
- The tombstone of a DID when the account rows cannot be written after
  the directory took the operation. The Candidate removes the actor store
  and reports the failure; the DID stays in the directory.
- `didDoc` in the session answers (`PDS_ENABLE_DID_DOC_WITH_SESSION`).
- Rate limits: `createSession` is limited by identifier and address at
  the pin, `createAccount` by address.
- The rest of unit 5: mail and email tokens, deactivation and deletion,
  admin, invite administration, handle and PLC changes, the proxy,
  service auth and OAuth.

## 4. Dependencies

`scrypt` with `pbkdf2` and `salsa20`, from RustCrypto. `cargo vet` has 305
exemptions, 5 more. The development profile builds `scrypt` and `salsa20`
with optimisation: without it one password check takes about two seconds
in a test.
