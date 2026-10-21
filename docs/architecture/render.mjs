// Renders every diagrams/*.mmd to svg/<name>.svg (light) and svg/<name>.dark.svg
// with beautiful-mermaid. Run `pnpm install && pnpm render` in this directory.
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { renderMermaidSVG, THEMES } from "beautiful-mermaid";

const src = new URL("./diagrams/", import.meta.url);
const out = new URL("./svg/", import.meta.url);
const args = process.argv.slice(2);
if (args.some((arg) => arg !== "--check")) {
  console.error("Usage: node render.mjs [--check]");
  process.exit(1);
}
const check = args.includes("--check");
if (!check) mkdirSync(out, { recursive: true });

const common = { font: "Inter, ui-sans-serif, system-ui, sans-serif", padding: 24, thoroughness: 5 };
const variants = [
  ["", THEMES["github-light"]],
  [".dark", THEMES["github-dark"]],
];

let failed = 0;
const expected = new Set();
for (const file of readdirSync(src).filter((f) => f.endsWith(".mmd")).sort()) {
  const text = readFileSync(new URL(file, src), "utf8");
  for (const [suffix, theme] of variants) {
    try {
      const svg = renderMermaidSVG(text, { ...theme, ...common });
      const name = file.replace(/\.mmd$/, `${suffix}.svg`);
      expected.add(name);
      const target = new URL(name, out);
      if (check) {
        if (!existsSync(target) || readFileSync(target, "utf8") !== svg) {
          failed += 1;
          console.error(`${name} is missing or stale; run pnpm render`);
        }
      } else {
        writeFileSync(target, svg);
      }
    } catch (e) {
      failed += 1;
      console.error(`${file}${suffix}: ${e.message}`);
    }
  }
  if (!check) console.log(`rendered ${file}`);
}
if (check && existsSync(out)) {
  for (const file of readdirSync(out).filter((name) => name.endsWith(".svg"))) {
    if (!expected.has(file)) {
      failed += 1;
      console.error(`${file} has no diagram source; remove the obsolete SVG`);
    }
  }
}
if (failed) process.exit(1);
if (check) console.log(`Checked ${expected.size} diagram SVGs`);
