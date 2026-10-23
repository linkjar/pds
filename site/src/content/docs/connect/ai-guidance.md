---
title: Use these docs with AI
description: Copy a guide or the full documentation into your chosen assistant, and keep setup checks grounded in the current server.
---

Use **Copy page** above any guide to copy its Markdown, including the source
URL and command examples. **Use with AI** also offers the full documentation
and a link to the current page. Code blocks have their own copy buttons.

## Choose how much context to share

| You need | Use |
|---|---|
| Help with one task | Copy that guide, such as [native installation](/start/native/) |
| Help finding the right guide | [Documentation index](/llms.txt) |
| Context across setup and operations | [Complete documentation](/llms-full.txt) |

The complete export can be large. Start with the relevant page when your
assistant has a small context window. If copying is unavailable in your
browser, use **View Markdown** and copy the text directly.

## A starting prompt

Copy this prompt and the relevant installation guide into your chosen assistant:

```text title="Plan a PDS installation"
Help me set up LinkJar PDS using the attached official documentation.
First ask whether this is a new installation or an existing data directory.
Ask for my OS, CPU architecture, domain, and preferred installation method.
Use the documented settings and commands. Do not invent an installer command.
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

A guided Rust installer is not shipped yet. The supported setup paths are
[NixOS or a native binary](/start/native/) and [Docker Compose](/start/install/).
Neither requires an LLM or a model-provider account.
