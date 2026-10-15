---
title: Handles and certificates
description: Hosted handles under the server's name, handles on a domain of the account's own, and how Caddy decides which names get a certificate.
sidebar:
  order: 4
---

A handle is a domain name that resolves to an account's DID. The server
hosts handles under its own name, and an account can use a domain of its
own instead.

## Hosted handles

`PDS_SERVICE_HANDLE_DOMAINS` lists the suffixes, each with a leading dot
and separated by commas. The default is the server's own name, so handles
are `alice.pds.example.com`. Each suffix needs a wildcard DNS record at
the host and a certificate for each handle.

To offer a second suffix, such as `.example.org`:

1. Add `.example.org` to `PDS_SERVICE_HANDLE_DOMAINS`:
   `.pds.example.com,.example.org`.
2. Point `*.example.org` at the host.
3. Add the names to the site block of the Caddyfile:
   `{$PDS_HOSTNAME}, *.{$PDS_HOSTNAME}, *.example.org`.

The server answers `/.well-known/atproto-did` for each hosted handle by
the request's `Host` header. Handles cannot end in `.alt`, `.arpa`,
`.example`, `.internal`, `.invalid`, `.local`, `.localhost` or `.onion`,
and the reference server's reserved names are refused.

## Certificates on demand

Caddy does not hold a wildcard certificate. It asks Let's Encrypt for a
certificate the first time a client connects to a name, and before it
asks, it calls the server's `/tls-check?domain=<name>`. The server says
yes for its own name and for the handle of each active account, and no
for every other name. A stranger who points a name at your host gets no
certificate from it, and the server never uses up the rate limits of
Let's Encrypt on names it does not host.

The first request for a new handle waits a few seconds for the
certificate. A deactivated or deleted account's handle gets no new
certificate.

## A handle on the account's own domain

An account can use `alice.com` as its handle. Alice proves the domain is
hers in one of two ways, then changes the handle in the app's settings:

- A DNS record `_atproto.alice.com` of type `TXT` with the value
  `did=did:plc:…`, or
- the file `https://alice.com/.well-known/atproto-did` with the DID as
  its body.

The server checks DNS first and the file second; when both exist and
disagree, DNS wins. The old hosted handle is released. The AT Protocol's
[handle guide](https://atproto.com/guides/identity#handles) has the detail
for people on your server.

:::note[Keep the server off a shared domain]
Serve the PDS on a domain of its own, not on a subdomain of a site that
also serves a web app with an OAuth client. The blobs and the
authorization pages of the server would otherwise share a parent domain
with the app's cookies.
:::
