---
title: Profiles
description: The stock profile reproduces the reference server. The linkjar profile adds LinkJar's sign-in providers and policies.
sidebar:
  order: 5
---

Behaviour that varies per operator sits behind extension points in the
server, and `PDS_PROFILE` selects a set of them. Everything else, from the
repository engine to the firehose, is the same in both profiles.

## `stock`

The default. The server behaves as the reference server does: sign-up with
an email address, a password and an invite code; sign-in with the password
or an app password, or through OAuth; the reference server's mail.

Most self-hosters want this one.

## `linkjar`

The profile LinkJar runs on its own hosts. It adds:

- Sign-in with Apple, Google and GitHub, from the sign-up and sign-in
  pages. Each provider needs `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID` and
  `PDS_EXTERNAL_<PROVIDER>_CLIENT_SECRET`, for `APPLE`, `GOOGLE` and
  `GITHUB`.
- A handle policy: a handle suggested from the display name or the email
  address, 3 to 18 characters, with a suffix on collision, and one rename
  of a hosted handle within 30 days.
- An account page at `/account` with the sign-in methods, the sessions and
  the devices of an account, and a receipt of the sign-up that the LinkJar
  apps read.
- LinkJar's mail templates, with a text alternative and no tracking.
- hCaptcha on the sign-up page when `PDS_HCAPTCHA_SITE_KEY`,
  `PDS_HCAPTCHA_SECRET_KEY` and `PDS_HCAPTCHA_TOKEN_SALT` are set. With
  hCaptcha on, new accounts come through the sign-up page only, and
  `createAccount` from the API is closed to them.
- Operator and moderator roles for the [console](/scale/sidecars/).

The [specification's section 13](https://github.com/linkjar/pds/blob/main/docs/SPEC.md#13-the-linkjar-profile)
has every rule. Switch profiles with a restart; the data directory is the
same for both.
