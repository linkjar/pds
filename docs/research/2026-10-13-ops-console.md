# Unit 9: the ops listener and the operations console

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §22.1](../SPEC.md#221-ops-listener) · [sidecars.md §1](../sidecars.md#1-the-operations-console)

Until this unit an operator had the admin password, the XRPC admin
methods and SQL. The server now has a private listener for operators,
and a console that uses it.

## 1. Result

| Part | State |
|---|---|
| Ops listener on `PDS_OPS_ADDRESS` | Built: 9 reads, 9 actions, one live stream, and the admin methods. |
| Operator roles | Built: `operator` and `moderator` in `ops.sqlite`, granted by the admin, in the `linkjar` profile. |
| Live firehose subscribers | Built: address, user agent, cursor, lag, frames, bytes, last frame, and a disconnect. |
| Blocked email domains | Built: the list, and the check in `createAccount`. |
| Audit of operator actions | Built: operator, credential, client, reason, `console`. |
| The console | Built as `pds-console`: sign-in through the PDS, six pages, nine actions. |
| Telemetry pane | Not built. |

The gate of the unit has two parts. "Every action lands in the audit log
with operator and reason": the tests read the entries. "The console runs
with no database access": the crate depends on `pds-identity` for the
DPoP key and on no storage crate.

## 2. How it is built

- **One credential check.** The listener takes the admin password, or an
  OAuth access token that the resource server of unit 5 accepts. For a
  token it then asks three things: the client is a console the
  configuration names, the profile grants roles, the account holds one.
- **Admin methods without the admin password.** The console must not
  hold the admin password. The listener therefore serves the admin
  methods itself: it checks the operator, replaces the credential of the
  request with the admin credential, and runs the method. A task-local
  value carries the operator to the audit entry.
- **Subscribers.** The firehose handler registers each connection. The
  registry holds counters that the handler updates after each frame, and
  a notifier that ends the connection with a close frame.
- **The console as a loopback client.** A loopback client needs no
  metadata document, so the console needs no public name. It makes a DPoP
  key for each sign-in, pushes the request, and exchanges the code. Its
  proofs name the public address of the PDS, also when it calls the ops
  listener on a private address.
- **Pages.** Rendered on the server, with every value of the PDS escaped.
  One script file reads `/live.json` every two seconds and writes the
  numbers into the page. The content security policy allows no inline
  script and no request to another origin.

## 3. How it is checked

- `crates/linkjar-pds/tests/accounts.rs`: the admin on the listener, with
  each read, each refusal, and the entries of the actions; a subscriber
  that the live stream shows and that the operator disconnects.
- `crates/linkjar-pds/tests/oauth.rs`: an account that gets a role, acts
  as a moderator, is refused the actions of an operator, and loses the
  role; a session of another client and the stock profile; and the
  console from the sign-in to the sign-out, with a browser that keeps its
  cookie.
- `crates/pds-console`: the configuration, a DPoP proof, the sessions,
  the escaping of the pages, and the plan of each form action.
- `crates/pds-storage` and `crates/pds-xrpc`: the roles, the blocked
  domains, the registry, the role order and the reason.

## 4. What differs from the design

1. **The bearer token.** [sidecars.md](../sidecars.md) §1.4 named a
   bearer token that the operator provisions. The listener takes the
   operator's own OAuth session, so an action has a person as its actor.
2. **The reason** is a header, `X-Ops-Reason`, on every action. The
   bodies of the admin methods are the lexicon's and have no place for
   it.
3. **The live view** of the console is a request every two seconds. The
   ops listener has the WebSocket stream; the console does not use it.

## 5. What is not built

1. The telemetry pane and the OTLP receiver of the console, and the OTLP
   export of the server.
2. A console under a host name (see [sidecars.md](../sidecars.md) §1.6).
3. Upstream health, relay crawl status, job state, and the result of the
   last `verify` on an account card.
4. Rate multipliers, blocked IP ranges, a rate cap for a subscriber, and
   the queue depth of a subscriber.
5. `repair`, blob garbage collection and log compaction as triggers.
6. Console forms for record and blob takedowns, forced changes of an
   account, deletion and invite codes. The listener serves the methods.
7. The Harness has no scenario for the ops listener: the Reference has
   none to compare with.

## 6. Dependencies

None is new. `pds-console` uses crates the workspace had. `cargo vet`
needs no new exemption.
