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
