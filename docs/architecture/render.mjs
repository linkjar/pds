// Renders every diagrams/*.mmd to svg/<name>.svg (light) and svg/<name>.dark.svg
// with beautiful-mermaid. Run `pnpm install && pnpm render` in this directory.
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { renderMermaidSVG, THEMES } from "beautiful-mermaid";

const src = new URL("./diagrams/", import.meta.url);
const out = new URL("./svg/", import.meta.url);
mkdirSync(out, { recursive: true });

const common = { font: "Inter, ui-sans-serif, system-ui, sans-serif", padding: 24, thoroughness: 5 };
const variants = [
  ["", THEMES["github-light"]],
  [".dark", THEMES["github-dark"]],
];

let failed = 0;
for (const file of readdirSync(src).filter((f) => f.endsWith(".mmd")).sort()) {
  const text = readFileSync(new URL(file, src), "utf8");
  for (const [suffix, theme] of variants) {
    try {
      const svg = renderMermaidSVG(text, { ...theme, ...common });
      writeFileSync(new URL(file.replace(/\.mmd$/, `${suffix}.svg`), out), svg);
    } catch (e) {
      failed += 1;
      console.error(`${file}${suffix}: ${e.message}`);
    }
  }
  console.log(`rendered ${file}`);
}
if (failed) process.exit(1);
