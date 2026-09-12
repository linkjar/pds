# Hosting policy content inventory — unpublished

This is the technical content for #94's future terms, privacy and support pages.
It is not the published hosting agreement. Operator identity, contact/postal
address, jurisdiction and a monitored support email have been requested. Also
complete the retention schedule, subprocessors/locations, applicable legal
bases and rights process, acceptable-use/enforcement terms, and effective date
before publishing the pages or setting their PDS URLs.

## Service and account controls

LinkJar's hosted PDS stores the account repository and blobs and participates in
AT Protocol identity and synchronization. Its operator has the PDS's online
repository/identity signing material; the account's offline recovery custody is
a separate operational responsibility. The hosting agreement needs to identify
the operator, available service, account responsibilities, acceptable use,
suspension/deletion procedure, decision review and contact route.

Users should be able to export their repository as CAR plus referenced blobs
and move to another compatible PDS while preserving their DID. Private Jar
recovery material is needed separately to decrypt private content after moving.
Do not advertise a completed guided move-in experience before #103 is accepted.

## Privacy boundary to explain

The PDS operator can process account email (including relay email), external
provider identity/subject and related authentication state, handles and DIDs,
network addresses and request metadata, public repository records and blobs,
encrypted private records/blobs, and their sizes and timing. Operational mail,
CAPTCHA, storage and monitoring providers process the data needed for their
configured roles; identify the actual services and locations in the final policy.

Private Jar clients encrypt private content before upload. The ordinary PDS
storage path does not receive the Private Jar seed or plaintext encrypted
content. This does not hide public data, traffic metadata or account identity,
and does not promise protection against a compromised client/device or malicious
client software. Distinguish PDS hosting from any separately enabled app AI,
capture, analytics or public-indexing services in the complete app privacy notice.

## Deletion, portability and support

Describe the authenticated account-delete request and confirmation procedure,
the actual delay before deletion, token/session handling, blob cleanup, backup
expiration and any documented retention obligations. Deleting an account cannot
guarantee removal of copies already held by independent relays or other users.
Apple grant/token revocation is still pending #100; do not describe it as shipped.

Publish a working support route for sign-in/reset, handle changes, export/move,
deletion and abuse reports. Explain identity verification without collecting
passwords or recovery material. State response expectations only after someone
has accepted responsibility for the mailbox.
