# Fuzz targets

`cargo +nightly fuzz run <target>` with cargo-fuzz. Every parser of untrusted
bytes gets a target (docs/SPEC.md section 14.5); the nightly workflow runs
each for ten minutes and keeps the corpus between nights.

| Target | Input | Beyond "does not panic" |
|---|---|---|
| `syntax_*` | A handle, NSID, DID, AT URI, datetime or TID | |
| `data_json` | JSON into the data model | Encodes to DAG-CBOR and decodes to the same value. |
| `data_cbor` | DAG-CBOR into the data model | Whatever decodes is canonical: it encodes back to the input. The limit check agrees with the decoder. |
| `repo_car` | A CAR, read as an import reads it | A walk never enters more nodes than the CAR has blocks. A tree that is accepted is the canonical tree of its keys. |
| `repo_node` | A tree node block or a commit block | Whatever decodes encodes back to the input. |
| `identity_did_doc` | A DID document; a TXT answer; a `/.well-known/atproto-did` body | |
| `identity_key` | A `did:key`, a Multikey, a DID, a signature | A `did:key` that parses has one spelling. |
| `lexicon_record` | A record as JSON | Checked against the data model rules and the interop lexicon catalog. |
| `xrpc_request` | A query string, a `listRepos` cursor, an `Authorization` header | A refused query is 400 with the method's prefix. A cursor that parses gives a time and a DID that is not empty. |
| `xrpc_body` | The JSON body of a write procedure and the record in it | The body checks, the record validation and the wording of each refusal hold for any input. |

`repo_car` and `repo_node` start from the CAR files in
`crates/pds-repo/tests/fixtures`, which hold real repositories and the
shared-node trees of upstream's `mst-dag.test.ts`.
