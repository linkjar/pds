---
title: Use these docs with AI
description: Use llms.txt, focused Markdown bundles, or a single guide with your chosen assistant.
---

Use **Copy page** above any guide to copy its Markdown, including the source
URL and command examples. **Use with AI** offers the documentation index,
the full documentation, and a link to the current page. Code blocks have their
own copy buttons; copying a command does not include the language or file title.

These controls copy public documentation to your clipboard. They do not send
it to an AI provider. Choose what to paste into your assistant.

## Choose how much context to share

| You need | Use |
|---|---|
| Help with one task | Copy that guide, such as [native installation](/start/native/) |
| Help finding the right guide | [Documentation index](/llms.txt) |
| A new server or a different installation method | [Setup bundle](/llms-setup.txt) |
| Backups, updates, migration, or troubleshooting | [Operations bundle](/llms-operations.txt) |
| Connecting an AI client or the operations console | [Clients bundle](/llms-clients.txt) |
| Context across the whole project | [Complete documentation](/llms-full.txt) |

The complete export can be large. Start with the relevant page when your
assistant has a small context window. If copying is unavailable in your
browser, use **View Markdown** and copy the text directly.

## Give an assistant the documentation URL

If your assistant can fetch web pages, give it the index URL. It can choose
the relevant guides from there:

```text title="Documentation index"
https://pds.linkjar.io/llms.txt
```

If it cannot fetch URLs, download a bundle and attach the file, or paste the
copied text. This example saves the setup guides:

```bash title="Download the setup bundle"
curl --fail --location --output linkjar-pds-setup.md \
  https://pds.linkjar.io/llms-setup.txt
```

Every guide also has a **View Markdown** link. For example,
[`/start/native.md.txt`](/start/native.md.txt) is the native installation guide.
The exports include source links and language-labeled code blocks. They are
generated from the same content as this site when it is built.

The HTML identifies each page's Markdown alternative and links to `llms.txt`
for tools that support discovery. Support varies by assistant; supplying the
URL or file directly is the most reliable way to provide the context.

## A starting prompt

Copy this prompt and the relevant installation guide into your chosen assistant:

```text title="Plan a PDS installation"
Help me set up LinkJar PDS using the attached official documentation.
First ask whether this is a new installation or an existing data directory.
Ask for my OS, CPU architecture, domain, and preferred installation method.
Use the documented settings and commands, and cite the relevant guides.
Explain which checks are local and which require testing from another host.
Prepare a plan before changing files, services, DNS, or firewall rules.
Keep credentials and private keys out of the conversation and logs.
Generate and store secrets locally using the documented protected files.
Stop if existing data or an unsupported platform changes the plan.
After setup, verify configuration, HTTPS, health, and server metadata.
Explain what remains unverified, including email delivery and restore tests.
```

Share redacted error messages when you need help. Keep runtime environment
files, passwords, private keys, and account exports on the host or in your
secret manager. Review the assistant's proposed commands before running them.

## Checks you can run today

The server already provides `check-config` and `health`. Use the
[operator-command helper](/operate/cli/) to run them with the service's actual
environment. Then follow [verify the setup](/start/verify/) for external HTTPS
and metadata checks.

`check-config` validates configuration and required serving secrets. It does
not prove that DNS, inbound networking, SMTP delivery, or backups work.

## Installation help and MCP access

These are separate uses of AI. An assistant can help you follow the manual
installation guide. The [MCP sidecar](/connect/mcp/) connects an AI client to
records in an account after sign-in; it has no host-installation tools.

The supported setup paths are [NixOS or a native binary](/start/native/) and
[Docker Compose](/start/install/). Neither requires an LLM or a model-provider
account.
