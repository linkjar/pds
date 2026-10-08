import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { chromium } from "playwright";
const localFixture = process.env.EXTERNAL_UI_BASE
  ? undefined
  : await import("./external-ui-server.mjs");
const origin = process.env.EXTERNAL_UI_BASE || localFixture.origin;
const requestUri =
  "urn:ietf:params:oauth:request_uri:req-01234567890123456789012345678901";
const ephemeralToken =
  "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkaWQ6cGxjOmFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYSJ9.c2lnbmF0dXJl";
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({
    locale: "en-US",
    viewport: { width: 390, height: 844 },
  });
  await page.goto(
    `${origin}/oauth/authorize?request_uri=${encodeURIComponent(requestUri)}&prompt=create`,
    { waitUntil: "networkidle" },
  );
  // 102-signup-journey: the email form is open and first; the providers
  // follow it under an "or" divider.
  await page
    .getByRole("heading", { level: 1, name: "Create your account." })
    .waitFor();
  assert.equal(await page.locator("details").count(), 0);
  assert.equal(
    await page.getByLabel("Email", { exact: true }).isVisible(),
    true,
  );
  const password = page.getByLabel("Password", { exact: true });
  assert.equal(await password.isVisible(), true);
  assert.equal(await password.getAttribute("minlength"), "8");
  assert.equal(await password.getAttribute("autocomplete"), "new-password");
  await page.getByText("At least 8 characters.", { exact: true }).waitFor();
  for (const label of ["Apple", "Google", "GitHub"])
    assert.equal(
      await page.getByRole("link", { name: `Continue with ${label}` }).count(),
      1,
    );
  assert.equal(
    await page.evaluate(() => {
      const form = document.querySelector("form");
      const divider = [...document.querySelectorAll("p")].find(
        (p) => p.textContent === "or",
      );
      const apple = document.querySelector(
        'a[href^="/oauth/external/apple/start?request_uri="]',
      );
      const follows = (a, b) =>
        Boolean(
          a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING,
        );
      return follows(form, divider) && follows(divider, apple);
    }),
    true,
  );
  await page
    .getByText("By continuing you agree to the", { exact: false })
    .waitFor();
  await mkdir(".build", { recursive: true });
  await page.screenshot({ path: ".build/external-signup.png", fullPage: true });
  // Continue leads to the handle step; the providers belong to the first step.
  await page.getByLabel("Email", { exact: true }).fill("new@example.com");
  await password.fill("correct-horse-battery");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.locator('input[name="handle"]').waitFor();
  assert.equal(
    await page.getByRole("link", { name: "Continue with Apple" }).count(),
    0,
  );
  await page.goto(
    `${origin}/oauth/authorize?request_uri=${encodeURIComponent(requestUri)}`,
    { waitUntil: "networkidle" },
  );
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  for (const label of ["Apple", "Google", "GitHub"])
    assert.equal(
      await page.getByRole("link", { name: `Continue with ${label}` }).count(),
      1,
    );
  assert.equal(
    await page.getByLabel("Password", { exact: true }).isVisible(),
    false,
  );
  await page.locator("summary").click();
  assert.equal(
    await page.getByLabel("Password", { exact: true }).isVisible(),
    true,
  );
  await page.screenshot({ path: ".build/external-signin.png", fullPage: true });
  console.log(
    "PASS compiled sign-up email form first with providers below, sign-in provider buttons over a collapsed email form, mobile viewport",
  );
  await page.goto(`${origin}/account/sign-in`, { waitUntil: "networkidle" });
  for (const label of ["Apple", "Google", "GitHub"]) {
    const link = page.getByRole("link", { name: `Continue with ${label}` });
    assert.equal(await link.count(), 1);
    assert.equal(
      (await link.getAttribute("href")).includes("request_uri"),
      false,
    );
  }
  console.log(
    "PASS compiled standalone account sign-in offers external reauthentication without a PAR",
  );
  for (const mode of ["new", "existing", "deactivated"]) {
    await page.goto(
      `${origin}/oauth/external/google/complete?state=secret&mode=${mode}&deactivated=${mode === "deactivated" ? "1" : "0"}`,
      { waitUntil: "networkidle" },
    );
    assert.equal(new URL(page.url()).pathname, "/oauth/authorize");
    assert.equal(
      new URL(page.url()).searchParams.get("request_uri"),
      requestUri,
    );
    assert.equal(new URL(page.url()).searchParams.has("state"), false);
    if (mode === "deactivated") {
      const response = page.waitForResponse((r) =>
        r.url().endsWith("/reactivate-account"),
      );
      await page
        .getByRole("button", {
          name: "Yes, reactivate my account",
          exact: true,
        })
        .click();
      assert.equal((await response).status(), 200);
    }
    const response = page.waitForResponse((r) => r.url().endsWith("/consent"));
    await page.getByRole("button", { name: "Authorize", exact: true }).click();
    assert.equal((await response).status(), 200);
    const call = (await response).request().headers();
    assert.equal(new URL(call.referer).pathname, "/oauth/authorize");
    assert.equal(
      new URL(call.referer).searchParams.get("request_uri"),
      requestUri,
    );
    assert.equal(call.authorization, `Bearer ${ephemeralToken}`);
  }
  console.log(
    "PASS compiled callback page canonicalizes before real consent/reactivation API guards and preserves ephemeral binding",
  );
} finally {
  await browser.close();
  if (localFixture) await localFixture.close();
}
