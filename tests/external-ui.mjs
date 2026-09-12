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
  for (const label of ["Apple", "Google", "GitHub"])
    assert.equal(
      await page.getByRole("link", { name: `Continue with ${label}` }).count(),
      1,
    );
  assert.equal(await page.locator("details").getAttribute("open"), null);
  assert.equal(
    await page
      .getByLabel("Handle", { exact: true })
      .isVisible()
      .catch(() => false),
    false,
  );
  await page.locator("summary").click();
  assert.notEqual(await page.locator("details").getAttribute("open"), null);
  await mkdir(".build", { recursive: true });
  await page.screenshot({ path: ".build/external-signup.png", fullPage: true });
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
    "PASS compiled sign-up and sign-in provider buttons, collapsed email forms, mobile viewport",
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
