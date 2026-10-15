//! The LinkJar journey: the look of every page the server renders.
//!
//! The pages take the look of LinkJar's onboarding, as the patched
//! Reference build had it (patch 102, the sign-up journey): a dark stage with three soft colour
//! blooms, Geist, a light pill for the main action, filled fields and
//! full-width provider rows. The values come from the app repository's
//! prototype (`apps/web/src/styles/prototype/social.css`), which also
//! draws the account page: a list of sections at the side, a hero card,
//! three tiles, then grouped rows with a pill action each.
//!
//! [`TOKENS`] is the one block of design tokens. The console takes the
//! same block, so a change of colour here is a change of colour there.
//! [`RULES`] styles the markup of [`crate::pages`]. [`asset`] serves the
//! typeface and the marks, which the pages load from the server itself.

/// The design tokens, as custom properties on `:root`.
///
/// The inks, the hairline, the fills, the accent, the focus ring, the
/// good and warning tints, the stage, the ink of the light pill, the
/// easing and the typeface.
pub const TOKENS: &str = ":root{\
--lj-ink:#f2f5fa;--lj-ink-2:#aab6c9;--lj-ink-3:#8494ad;\
--lj-line:#c4d6f51f;--lj-fill:#c9dcff0b;--lj-fill-2:#c9dcff14;\
--lj-accent:#bfd4f6;--lj-focus:#b2cdf5;--lj-good:#9fe0c0;--lj-warn:#f0c9a0;\
--lj-stage:#05070b;--lj-card:#0c111b;--lj-pill-ink:#121b2a;\
--lj-ease:cubic-bezier(.2,.8,.2,1);\
--lj-font:Geist,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;\
color-scheme:dark}";

/// The path of the typeface, under which [`asset`] serves it.
pub const FONT_PATH: &str = "/oauth/assets/geist-latin.woff2";

/// The path of LinkJar's mark.
pub const LINKJAR_MARK_PATH: &str = "/oauth/assets/linkjar-mark.png";

/// The path of the neutral mark of the `stock` profile.
pub const STOCK_MARK_PATH: &str = "/oauth/assets/stock-mark.svg";

/// The typeface: Geist, Latin subset, variable weight, from
/// [`FONT_PATH`].
pub const FONT_FACE: &str = "@font-face{font-family:Geist;\
src:url(/oauth/assets/geist-latin.woff2) format('woff2');\
font-weight:100 900;font-style:normal;font-display:swap}";

/// The rules of the pages. One rule per line: the console reads them too.
pub const RULES: &str = r"
*{box-sizing:border-box}
html{background:var(--lj-stage)}
body{position:relative;isolation:isolate;display:flex;flex-direction:column;min-height:100svh;margin:0;overflow-x:clip;background:var(--lj-stage);color:var(--lj-ink);font:15px/1.55 var(--lj-font);-webkit-font-smoothing:antialiased}
body::before{content:'';position:fixed;inset:0;z-index:-1;pointer-events:none;background:radial-gradient(60% 55% at 50% 0,#2a3a7a40,transparent 70%),radial-gradient(42% 46% at 100% 100%,#4b63c026,transparent 70%),radial-gradient(40% 44% at 0 100%,#a0527a21,transparent 70%)}
a{color:inherit}
button,input,a{touch-action:manipulation;-webkit-tap-highlight-color:transparent}
p{margin:0 0 14px;color:var(--lj-ink-2)}
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.92em;overflow-wrap:anywhere}
.lj-brand{display:flex;align-items:center;gap:10px;width:max-content;padding:18px 28px;color:var(--lj-ink);font-size:18px;font-weight:550;letter-spacing:-.03em;text-decoration:none}
.lj-brand img{width:32px;height:32px;object-fit:contain}
.lj-stage{display:flex;flex:1;flex-direction:column;justify-content:center}
.lj-panel{width:min(100% - 40px,440px);margin:0 auto;padding:0 0 24px}
.lj-title{margin:0 0 20px;color:var(--lj-ink);font-size:clamp(34px,4vw,46px);font-weight:500;line-height:1.04;letter-spacing:-.045em;text-align:center;text-wrap:balance;overflow-wrap:anywhere}
.lj-lede{max-width:40ch;margin:-10px auto 26px;color:var(--lj-ink-2);font-size:17px;line-height:1.5;letter-spacing:-.01em;text-align:center;text-wrap:pretty}
.lj-lede strong,.lj-lede code{color:var(--lj-ink);font-weight:550}
.lj-lede a,.lj-fine a{color:var(--lj-ink-2);text-decoration:underline;text-underline-offset:3px}
h2{margin:28px 0 12px;color:var(--lj-ink);font-size:20px;font-weight:500;letter-spacing:-.025em;text-align:center}
.lj-steps{display:flex;flex-direction:column;align-items:center;gap:6px;margin:-8px 0 18px;color:var(--lj-ink-3);font-size:12px}
.lj-steps p{margin:0;color:inherit}
.lj-track{display:flex;width:72px;height:4px;overflow:hidden;border-radius:99px;background:var(--lj-fill-2)}
.lj-track i{flex:1}
.lj-track .done{background:linear-gradient(90deg,#d7e9fa,#b4bbea)}
.lj-field{margin:0 0 16px}
.lj-field label{display:block;margin:0 0 6px;color:var(--lj-ink-3);font-size:13px;font-weight:500}
input[type=text],input[type=email],input[type=password]{display:block;width:100%;min-width:0;height:52px;padding:0 14px;border:0;border-radius:14px;background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink);font:inherit;font-size:17px;transition:box-shadow 180ms ease,background 180ms ease}
input::placeholder{color:#5d6c86}
input[readonly]{color:var(--lj-ink-2)}
input:focus-visible{outline:0;background:#c9dcff1c;box-shadow:inset 0 0 0 1.5px var(--lj-accent),0 0 0 5px #a7c4f11f}
.lj-affix{display:flex;align-items:center;border-radius:14px;background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line);transition:box-shadow 180ms ease,background 180ms ease}
.lj-affix input{flex:1;background:none;box-shadow:none;padding-right:4px}
.lj-affix input:focus-visible{background:none;box-shadow:none}
.lj-affix:focus-within{background:#c9dcff1c;box-shadow:inset 0 0 0 1.5px var(--lj-accent),0 0 0 5px #a7c4f11f}
.lj-affix span{padding-right:14px;color:var(--lj-ink-3);font-size:17px;white-space:nowrap}
.lj-hint{margin:8px 0 0;color:var(--lj-ink-3);font-size:14px;line-height:1.45}
.lj-hint strong{color:var(--lj-ink-2);font-weight:500}
.lj-check{display:flex;align-items:center;gap:10px;min-height:44px;margin:-8px 0 10px;color:var(--lj-ink-2);font-size:14px;cursor:pointer}
.lj-check input{width:18px;height:18px;margin:0;accent-color:#b4bbea}
.lj-actions{display:grid;gap:10px;margin-top:4px}
.lj-primary{display:flex;align-items:center;justify-content:space-between;gap:14px;width:100%;min-height:58px;margin:0;padding:7px 8px 7px 28px;border:0;border-radius:99px;background:linear-gradient(180deg,#f8fafe,#d9e3f3);box-shadow:inset 0 1px 0 #fff,0 14px 34px -12px #bcd3ff59;color:var(--lj-pill-ink);font:inherit;font-size:17px;font-weight:600;letter-spacing:-.015em;text-align:left;cursor:pointer;transition:transform 200ms var(--lj-ease),filter 160ms ease}
.lj-chip{display:grid;place-items:center;flex:none;width:44px;height:44px;border-radius:50%;background:var(--lj-pill-ink);color:var(--lj-ink);transition:transform 320ms var(--lj-ease)}
.lj-chip svg{width:18px;height:18px}
.lj-secondary{display:inline-flex;align-items:center;justify-content:center;gap:10px;width:100%;min-height:52px;margin:0;padding:0 24px;border:0;border-radius:99px;background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink);font:inherit;font-size:16px;font-weight:500;text-align:center;text-decoration:none;cursor:pointer;transition:background 160ms ease,transform 200ms var(--lj-ease)}
.lj-secondary:disabled{opacity:.45;cursor:default}
.lj-small{min-height:44px;font-size:14px}
.lj-quiet{display:inline-flex;align-items:center;justify-self:center;min-height:44px;margin:8px 0 0;padding:0 16px;border:0;border-radius:99px;background:none;color:var(--lj-ink-2);font:inherit;font-size:14px;font-weight:500;text-decoration:none;cursor:pointer;transition:background 160ms ease,color 160ms ease}
.lj-providers{display:grid;gap:10px}
.lj-provider{display:flex;align-items:center;gap:14px;width:100%;min-height:58px;padding:0 20px;border-radius:18px;background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line),inset 0 1px 0 #ffffff14;color:var(--lj-ink);font-size:16px;font-weight:550;letter-spacing:-.01em;text-decoration:none;transition:background 160ms ease,transform 200ms var(--lj-ease)}
.lj-provider svg{flex-shrink:0;width:22px;height:22px}
.lj-divider{display:flex;align-items:center;gap:14px;margin:22px 0 14px;color:var(--lj-ink-3);font-size:13px}
.lj-divider::before,.lj-divider::after{content:'';flex:1;height:1px;background:var(--lj-line)}
.lj-email-fold{margin-top:4px}
.lj-email-fold>summary{display:flex;align-items:center;justify-content:center;gap:8px;min-height:44px;border-radius:99px;color:var(--lj-ink-2);font-size:14px;font-weight:500;list-style:none;cursor:pointer;transition:background 160ms ease,color 160ms ease}
.lj-email-fold>summary::-webkit-details-marker{display:none}
.lj-email-fold>summary::after{content:'+';color:var(--lj-ink-3)}
.lj-email-fold[open]>summary::after{content:'–'}
.lj-email-fold>form{margin-top:12px;padding:20px 16px 18px;border-radius:20px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line)}
.lj-fine{margin:16px 0 0;color:var(--lj-ink-3);font-size:12px;line-height:1.55;text-align:center;text-wrap:pretty}
[role=alert]{margin:0 0 18px;padding:12px 14px;border-radius:14px;background:#f0c9a01a;box-shadow:inset 0 0 0 1px #f0c9a04d;color:var(--lj-warn);font-size:14px;line-height:1.45;overflow-wrap:anywhere}
.lj-card{margin:0 0 10px;padding:16px 18px;border-radius:18px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line)}
.lj-card p{margin:0 0 12px;font-size:14px}
.lj-card strong{color:var(--lj-ink);font-weight:500}
.lj-scopes{display:grid;gap:8px;margin:0 0 22px;padding:0;list-style:none}
.lj-scopes li{padding:12px 14px;border-radius:14px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink-2);font-size:14px}
.lj-note{margin:0 0 14px;color:var(--lj-ink-3);font-size:13px;text-align:center;text-wrap:pretty}
.h-captcha{display:flex;justify-content:center;margin:0 0 16px}
.lj-footer{display:flex;flex-direction:column;align-items:center;gap:10px;padding:8px 20px 20px;color:var(--lj-ink-3);font-size:12px}
.lj-footer nav{display:flex;flex-wrap:wrap;justify-content:center;gap:2px 16px}
.lj-footer a{text-decoration:none}
.lj-locale{display:inline-flex;align-items:center;min-height:32px;padding:0 14px;border-radius:99px;box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink-2);font-size:13px;font-weight:500}
.lj-account{display:flex;align-items:flex-start;gap:40px;width:100%;max-width:1040px;margin:0 auto;padding:16px 24px 64px}
.lj-side{position:sticky;top:16px;flex:0 0 200px;display:grid;gap:2px}
.lj-side a{display:flex;align-items:center;gap:12px;min-height:44px;padding:0 14px;border-radius:14px;color:var(--lj-ink-2);font-size:15px;text-decoration:none;transition:background 160ms ease,color 160ms ease}
.lj-side svg{flex-shrink:0;width:20px;height:20px}
.lj-side a[aria-current]{background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink)}
.lj-content{flex:1;min-width:0}
.lj-content section{scroll-margin-top:16px}
.lj-content section+section,.lj-tiles+section{margin-top:56px}
.lj-content h2{margin:0 0 6px;font-size:28px;font-weight:550;letter-spacing:-.035em;text-align:left}
.lj-content h3{margin:24px 0 10px;color:var(--lj-ink-3);font-size:13px;font-weight:500}
.lj-content .lj-lede{max-width:56ch;margin:0 0 18px;font-size:16px;text-align:left}
.lj-content .lj-note{margin:12px 0 0;text-align:left}
.lj-hero{display:flex;align-items:center;gap:28px;padding:28px;border-radius:28px;background:radial-gradient(90% 140% at 100% 0,#4b63c040,transparent 65%),radial-gradient(70% 120% at 0 100%,#a0527a2e,transparent 65%),var(--lj-card);box-shadow:inset 0 0 0 1px #c4d6f51f,inset 0 1px 0 #dfeaff29,0 40px 80px -50px #000}
.lj-hero h1{margin:0;font-size:clamp(32px,4vw,44px);font-weight:500;line-height:1.04;letter-spacing:-.045em;overflow-wrap:anywhere}
.lj-hero-text{min-width:0}
.lj-eyebrow{margin:0 0 12px;color:var(--lj-ink-3);font-size:12px;font-weight:500;letter-spacing:.14em;text-transform:uppercase}
.lj-badges{display:flex;flex-wrap:wrap;gap:8px;margin:16px 0 14px}
.lj-badge{padding:5px 12px;border-radius:99px;background:var(--lj-fill-2);box-shadow:inset 0 0 0 1px var(--lj-line);color:var(--lj-ink-2);font-size:13px}
.lj-hero .lj-fine{margin:0;text-align:left}
.lj-tiles{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin-top:12px}
.lj-tile{display:grid;gap:4px;padding:16px 18px;border-radius:20px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line);text-decoration:none;transition:background 160ms ease,transform 200ms var(--lj-ease)}
.lj-tile span{color:var(--lj-ink-3);font-size:13px}
.lj-tile strong{font-size:16px;font-weight:550;letter-spacing:-.01em;overflow-wrap:anywhere}
.lj-group{margin:0;overflow:hidden;border-radius:20px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line)}
.lj-row{position:relative;display:flex;align-items:center;gap:14px;width:100%;min-height:68px;padding:12px 14px 12px 18px;color:var(--lj-ink)}
.lj-row+.lj-row::before{content:'';position:absolute;top:0;left:18px;right:0;height:1px;background:var(--lj-line)}
.lj-row:has(.lj-row-icon)+.lj-row::before{left:72px}
.lj-row form{flex-shrink:0;margin:0}
.lj-row-text{flex:1;min-width:0}
.lj-row-text strong{display:block;font-size:15px;font-weight:500;letter-spacing:-.01em;line-height:1.3;overflow-wrap:anywhere}
.lj-row-text small{display:block;margin-top:2px;color:var(--lj-ink-3);font-size:13px;line-height:1.4;text-wrap:pretty;overflow-wrap:anywhere}
.lj-row-text small:first-child{margin:0 0 2px}
.lj-row-icon{display:grid;place-items:center;flex-shrink:0;width:40px;height:40px;border-radius:12px;background:var(--lj-fill-2);color:var(--lj-accent)}
.lj-row-icon svg{width:20px;height:20px}
.lj-tag{margin-left:8px;padding:2px 8px;border-radius:99px;background:#9fe0c01f;color:var(--lj-good);font-size:11px;font-style:normal;font-weight:500;vertical-align:2px;white-space:nowrap}
.lj-row-action{display:inline-flex;align-items:center;flex-shrink:0;min-height:40px;margin:0;padding:0 16px;border:0;border-radius:99px;background:var(--lj-fill-2);color:var(--lj-ink);font:inherit;font-size:14px;font-weight:500;text-decoration:none;cursor:pointer;transition:background 160ms ease,transform 200ms var(--lj-ease)}
.lj-row-action:disabled{opacity:.45;cursor:default}
.lj-content .lj-actions{display:flex;flex-wrap:wrap;justify-content:flex-start;gap:12px;margin-top:16px}
.lj-content .lj-actions .lj-secondary{width:auto;min-height:48px;font-size:15px}
.lj-facts{display:grid;grid-template-columns:repeat(3,1fr);gap:10px}
.lj-facts>div{padding:18px;border-radius:20px;background:var(--lj-fill);box-shadow:inset 0 0 0 1px var(--lj-line)}
.lj-facts strong{display:block;margin-bottom:6px;font-size:16px;font-weight:550;letter-spacing:-.01em}
.lj-facts p{margin:0;color:var(--lj-ink-3);font-size:14px;line-height:1.5}
.lj-advanced{margin-top:16px;color:var(--lj-ink-3);font-size:13px}
.lj-advanced summary{display:inline-flex;align-items:center;min-height:44px;margin-left:-12px;padding:0 12px;border-radius:99px;color:var(--lj-ink-2);list-style:none;cursor:pointer;transition:background 160ms ease,color 160ms ease}
.lj-advanced summary::-webkit-details-marker{display:none}
.lj-advanced summary::after{content:'+';margin-left:8px;color:var(--lj-ink-3)}
.lj-advanced[open] summary::after{content:'–'}
.lj-advanced p{margin:0 0 10px;color:inherit;line-height:1.55}
.lj-advanced strong{color:var(--lj-ink-2);font-weight:550}
.lj-advanced code{color:var(--lj-ink-2);font-size:12px}
.lj-toast{position:fixed;left:50%;bottom:24px;z-index:5;width:max-content;max-width:min(560px,100% - 32px);margin:0;padding:14px 18px;border-radius:18px;background:#1a2233f2;box-shadow:inset 0 0 0 1px #c4d6f533,0 20px 50px -20px #000;color:var(--lj-ink);font-size:14px;line-height:1.45;transform:translateX(-50%);backdrop-filter:blur(16px)}
:is(.lj-primary,.lj-secondary,.lj-quiet,.lj-provider,.lj-brand,.lj-footer a,.lj-fine a,.lj-lede a,.lj-email-fold>summary,.lj-check input,.lj-side a,.lj-tile,.lj-row-action,.lj-advanced summary):focus-visible{outline:2px solid var(--lj-focus);outline-offset:3px}
:is(.lj-primary,.lj-secondary:not(:disabled),.lj-provider):active{transform:scale(.98);transition-duration:60ms}
.lj-row-action:not(:disabled):active{transform:scale(.96);transition-duration:60ms}
@media(hover:hover){
.lj-primary:hover{filter:brightness(1.06)}
.lj-primary:hover .lj-chip{transform:translateX(3px)}
.lj-secondary:not(:disabled):hover,.lj-provider:hover,.lj-tile:hover,.lj-row-action:not(:disabled):hover{background:#c9dcff1f}
.lj-quiet:hover,.lj-email-fold>summary:hover,.lj-side a:hover,.lj-advanced summary:hover{background:#c9dcff12;color:var(--lj-ink)}
.lj-fine a:hover,.lj-lede a:hover,.lj-footer a:hover{color:var(--lj-ink)}
}
@media(min-width:761px){.lj-footer{flex-direction:row;justify-content:center;gap:16px}}
@media(max-width:900px){
.lj-account{flex-direction:column;gap:0;padding:8px 16px 48px}
.lj-side{position:sticky;top:0;z-index:4;flex:none;display:flex;gap:4px;width:calc(100% + 32px);margin:0 -16px 16px;padding:8px 16px;overflow-x:auto;background:#05070be6;backdrop-filter:blur(16px);scrollbar-width:none}
.lj-side a{flex-shrink:0;min-height:40px;font-size:14px}
.lj-side svg{display:none}
.lj-content{width:100%}
.lj-content section{scroll-margin-top:72px}
.lj-tiles,.lj-facts{grid-template-columns:1fr}
}
@media(max-width:760px){
.lj-brand{padding:14px 16px 6px;font-size:16px}
.lj-brand img{width:28px;height:28px}
.lj-stage{justify-content:flex-start}
.lj-panel{width:min(100% - 32px,440px);padding-top:12px}
.lj-title{margin-bottom:18px;font-size:30px}
.lj-lede{font-size:15.5px;line-height:1.45;margin-bottom:18px}
.lj-providers{gap:8px}
.lj-provider{min-height:54px}
.lj-primary{min-height:56px}
.lj-hero{flex-direction:column;align-items:flex-start;gap:20px;padding:22px}
.lj-content h2{font-size:24px}
.lj-toast{bottom:16px}
}
@media(prefers-reduced-motion:reduce){*,*::before,*::after{transition-duration:.01ms!important}}
@media(prefers-contrast:more){:root{--lj-ink-2:#d5deec;--lj-ink-3:#c2cde0;--lj-line:#a6bad7}}
";

/// The typeface, Geist, Latin subset (`assets/NOTICE.md`).
const FONT: &[u8] = include_bytes!("../assets/geist-latin.woff2");

/// LinkJar's mark, 128 pixels square.
const LINKJAR_MARK: &[u8] = include_bytes!("../assets/linkjar-mark.png");

/// The neutral mark of the `stock` profile.
const STOCK_MARK: &[u8] = include_bytes!("../assets/stock-mark.svg");

/// The files under `/oauth/assets/`: the media type and the bytes of the
/// one named `name`, the last segment of the path.
///
/// Each lives under one name for good, so the server may tell a browser
/// to keep it for a year.
#[must_use]
pub fn asset(name: &str) -> Option<(&'static str, &'static [u8])> {
    let path = format!("/oauth/assets/{name}");
    match path.as_str() {
        FONT_PATH => Some(("font/woff2", FONT)),
        LINKJAR_MARK_PATH => Some(("image/png", LINKJAR_MARK)),
        STOCK_MARK_PATH => Some(("image/svg+xml", STOCK_MARK)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_assets_are_served_under_their_paths_and_the_rules_name_the_font() {
        assert_eq!(
            asset("geist-latin.woff2").map(|(kind, _)| kind),
            Some("font/woff2")
        );
        assert_eq!(
            asset("linkjar-mark.png").map(|(_, bytes)| bytes.len()),
            Some(15_151)
        );
        assert_eq!(
            asset("stock-mark.svg").map(|(kind, _)| kind),
            Some("image/svg+xml")
        );
        assert_eq!(asset("../Cargo.toml"), None);
        assert_eq!(asset(""), None);
        assert!(FONT_FACE.contains(FONT_PATH));
        assert_eq!(FONT.len(), 29_288);
        assert!(FONT.starts_with(b"wOF2"));
        assert!(TOKENS.starts_with(":root{--lj-ink:#f2f5fa;"));
    }
}
