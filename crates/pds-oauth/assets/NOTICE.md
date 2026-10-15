# Page assets

`geist-latin.woff2` is Geist, Latin subset, variable weight, by the Geist
Project Authors (https://github.com/vercel/geist-font). It came through
patch 102 of the reference-image build, `102-signup-journey.patch`, which
carried it into the patched Reference build; that build left the repository
on 2026-10-15. It is licensed under the SIL Open Font License 1.1;
`Geist-OFL.txt` is the licence text as the font ships it.

`linkjar-mark.png` is LinkJar's mark, the same file as
[`parity/src/fixtures/linkjar-128.png`](../../../parity/src/fixtures/linkjar-128.png).
`stock-mark.svg` is a neutral mark drawn here for the `stock` profile.

`src/theme.rs` embeds the four files. The server serves them under
`/oauth/assets/` with a one-year cache lifetime, so a changed file needs a
new name.
