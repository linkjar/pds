# Sign-up journey

`102-signup-journey.patch` puts the email form first on the OAuth sign-up page and
gives the authorization screens the look of LinkJar's onboarding journey. It
belongs to the `linkjar/linkjar.io` onboarding build (October 2026) and applies
after `101-mail-templates.patch`. No upstream issue is filed.

The app sends a new member to `/oauth/authorize` with `prompt=create`. That page
was the stock card, with provider buttons first and the email form folded under
"Or use email". Next to the app's dark onboarding it looked like another product.

## What changed

The sign-up page opens on Email and Password, with the hint "At least 8
characters." and a Continue pill. The configured providers follow as full-width
rows under an "or" divider. The fine print "By continuing you agree to the Terms
of Service and the Privacy Policy." keeps the configured links and covers both
paths, so it appears once, on this first step. The handle step and the optional
hCaptcha step follow; the handle check still runs before the account is created.
Without hCaptcha the handle step is the last step. It now awaits account
creation, so a refused sign-up (for example, an email already in use) shows there
instead of failing silently.

`AuthShell` draws every authorization screen (sign-up, sign-in, welcome, consent,
password reset, reactivation, errors) as one centred column on a `#05070b` page
with three colour blooms. All text is Geist. The main action is a 58px light pill
with the arrow in a dark chip; provider rows are 58px, fields 52px, both on
`#c9dcff14`; fine print is 12px `#8494ad`. The values come from the app
repository's prototype stylesheet, `apps/web/src/styles/prototype/social.css`.
They live in one unlayered "LinkJar journey" section of `src/style.css`, so the
shared `components/ui/*` primitives stay unedited.

The stage is dark in both colour schemes, and the hCaptcha widget uses its dark
theme. The branding palette still colours errors and notices but not the main
button, and `branding.background` images are not drawn. Sign-in keeps its flow:
providers first, the email form folded under them. The account manager pages
keep the stock look.

Geist (Latin subset, variable weight) ships in
`packages/oauth/oauth-provider-ui/src/assets/fonts/` beside its SIL Open Font
License 1.1 text, and Vite emits it as a hashed asset. The provider adds
`font-src 'self'` to the page CSP and lets its assets route answer
`Sec-Fetch-Dest: font`. Nothing loads from another host.

New strings: "Create your account.", "At least # characters.", "or" and the new
fine print. "We're so excited to have you join us!", "Your account" and the old
disclaimer are gone. As in the earlier patches, English is filled and the other
six catalogs fall back to it. The browser tab still reads "Sign up".

## Tests

The image build still runs the patched provider tests. In CI,
`tests/external-ui.mjs` checks that the sign-up form is open and precedes the
divider and the provider links, the password hint and constraints, and that
Continue leads to the handle step; sign-in still needs the folded form.
`tests/provider-branding.mjs` checks, in light and dark, the `#05070b` page,
Geist loaded under the page CSP, the 58px pill with dark ink, the logo, the
policy links and no horizontal scroll.

On 2026-10-08 the ARM64 production image built with all seven patches, and its
provider tests and the CI image checks passed locally. A real PAR request with
`prompt=create`, LinkJar branding and placeholder Apple and Google credentials
produced the [phone](signup-journey.png) and [desktop](signup-journey-desktop.png)
screenshots. The CSP carried `font-src 'self'`, Geist loaded and the console
logged no errors. No provider sign-in, hCaptcha challenge or deployment ran.

## Runtime

No database, lexicon, configuration or API changes. The compiled UI and the page
CSP change. Ship it as a normal image update; switching the image restarts the
PDS. Check the sign-up page through a real PAR request afterwards.

## Removal

Remove the patch when upstream's OAuth UI supports a configurable layout and
typeface, or when LinkJar serves its own sign-up pages. Removal restores the
stock card and the folded email form; no data changes.

## Invitation hand-off

`103-invite-handoff.patch` applies after this patch. The production PDS keeps
`PDS_INVITE_REQUIRED=true`, but a visitor who arrives through a LinkJar.social
invitation must not see the "Invite code" field. The app hands the code to the
sign-up page in the URL fragment, and the page uses it without showing it. No
upstream issue is filed.

### Contract for the app

The app runs PAR as today, with `prompt=create`, and sends the visitor to the
normal authorize URL with this fragment:

```
https://pds.linkjar.social/oauth/authorize?client_id=…&request_uri=…#invite=<code>
```

`<code>` is the invite code passed through `encodeURIComponent`. Nothing else
goes in the fragment, and the app does not add `#step=`: `prompt=create` already
opens the sign-up step. The fragment never reaches the server.

### How the page keeps the code

The upstream page records its step in the fragment, as `#step=sign-up`, with
`history.replaceState` (`src/lib/location-step.ts`). It also rewrites the URL at
load when the path or `request_uri` is missing. The page therefore reads
`invite` at load, before either write, and removes it from the URL. The step
routing then finds an empty fragment and `prompt=create` decides the step, as
before. The page reads the fragment as URL-encoded parameters and removes only
`invite`, so a fragment such as `#step=sign-up&invite=<code>` would also work.
The app should still send `#invite=<code>` alone.

The code stays in memory and in `sessionStorage` on the PDS origin, under the
key `linkjar-invite-handoff`, together with the request URI. A reload of the
authorize page in the same tab restores it. The page deletes the code once the
account exists, once the server refuses the code, and when the request ends with
consent or cancellation. A provider sign-up can return straight to the app
without the page; its stored code then stays until the tab closes or loads
another request. A page load for another request URI deletes it, so it is never
used for another request.

### Sign-up form

With a handed-off code and invites required, the "Invite code" field is not
rendered. A hidden input submits the code with the email and password. The
email field takes the focus, and the steps are numbered as without the field.
Without a handed-off code the page is unchanged.

If the server refuses the code (unknown, disabled or used up), the wizard
returns to its first step. The "Invite code" field is back, focused and empty,
under the server's error, "The invite code is not valid". The email, password
and username keep their values, and the visitor can type another code. The
refused code is forgotten, so a reload also shows the field. The patch adds no
new strings. In one narrow case, the PDS reports a generic error instead: when
another sign-up uses up the code during this one. Submitting again then gets the
refusal and brings the field back.

### Apple and Google

A handed-off code also lets an invited visitor create an account through a
provider. The provider links on the sign-up page add it to the same-origin start
request as `invite_code`. The start keeps it in the existing flow record, which
is already bound to the device and the authorization request, and the completion
consumes it with that record. While invites are required, a new external account
needs that code. The PDS checks the code and records its use in the same account
transaction as an email sign-up. The state, nonce, PKCE and device checks do not
change, and the provider never receives the code. The start URL does contain the
code, so a proxy access log can record it.

Without a code, a new external account is refused as before, and known
identities sign in as before. If the PDS refuses the code, the completion shows
the error page with "The invite code is not valid". The visitor can then go back
to the app or sign up with email, where the field comes back. Only a handed-off
code reaches the providers: a code typed into the field does not.

### Tests

The image build runs `create-external-middleware.test.ts`. A new test checks
that, while invites are required, a new external account needs the code that its
start carried, that the store receives the code, and that the error page names a
refused code.

`scripts/browser-smoke.mjs` checks the production image after a real PAR request
with `prompt=create`. Without a hand-off, the field is first and has the focus.
With `#invite=`, the field is gone, the email field has the focus, the URL ends
in `#step=sign-up`, the Google link carries the code, and a reload keeps the
code. A sign-up with an unknown code then reaches the real PDS, which refuses it
before any account or PLC operation. The page shows the field, focused and
empty, with "The invite code is not valid", and a reload still shows the field.
For these API calls, `scripts/smoke.py` starts the production container on a port
that matches its OAuth issuer. It also configures a placeholder Google client,
which is never contacted.

On 2026-10-08 the ARM64 production image built locally with all eight patches.
Its build-time provider tests passed (109), as did the full `oauth-provider`
suite (127), the UI type check and its unit tests (100), the CI image checks and
`scripts/smoke.py --browser`. No account was created and no deployment ran.

### Runtime and removal

No database, lexicon or configuration change. The compiled UI changes, and the
external start route accepts an optional `invite_code` query parameter. Ship it
as a normal image update.

Remove the patch when the app stops handing off codes, or when sign-up opens
without invites (`PDS_INVITE_REQUIRED=false`). Removal shows the field to invited
visitors again; no data changes.
