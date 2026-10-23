import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import test from 'node:test';

const output = new URL('../../dist/', import.meta.url);

async function page(path) {
	return readFile(new URL(path, output), 'utf8');
}

test('installation Markdown preserves both labeled deployment paths and executable examples', async () => {
	const markdown = await page('start/install.md.txt');
	assert.match(markdown, /^# /);
	assert.match(markdown, /Published image/);
	assert.match(markdown, /Build from source/);
	assert.match(markdown, /```(?:sh|bash|shell)/);
	assert.match(markdown, /docker compose/);
	assert.doesNotMatch(
		markdown,
		/<\/?(?:Tabs|TabItem|Steps|FileTree)\b|^import /m,
	);
});

test('architecture Markdown describes each diagram once and keeps its caption', async () => {
	const markdown = await page('reference/architecture.md.txt');
	assert.equal((markdown.match(/!\[System context:/g) ?? []).length, 1);
	assert.equal((markdown.match(/!\[Storage:/g) ?? []).length, 1);
	assert.equal((markdown.match(/!\[Topologies:/g) ?? []).length, 1);
	assert.match(markdown, /A single node is the usual deployment\./);
	assert.doesNotMatch(markdown, /<Diagram\b|^import /m);
});

test('copied Markdown identifies its public source and keeps links, images, and anchors portable', async () => {
	const architecture = await page('reference/architecture.md.txt');
	assert.match(
		architecture,
		/\[Source\]\(https:\/\/pds\.linkjar\.io\/reference\/architecture\/\)/,
	);
	assert.match(
		architecture,
		/\]\(https:\/\/pds\.linkjar\.io\/_astro\/02-system-context\.[^)]+\.svg\)/,
	);
	assert.match(
		architecture,
		/\[Back up and restore\]\(https:\/\/pds\.linkjar\.io\/operate\/backup\/\)/,
	);
	const overview = await page('start/overview.md.txt');
	assert.match(
		overview,
		/\]\(https:\/\/pds\.linkjar\.io\/operate\/update\/#roll-back\)/,
	);
	assert.match(
		overview,
		/\[AT Protocol documentation\]\(https:\/\/atproto\.com\/\)/,
	);
	assert.doesNotMatch(await page('llms-full.txt'), /\]\((?:\/|#)/);
});

test('documentation pages offer page copying, complete context, and a Markdown fallback', async () => {
	for (const slug of [
		'start/overview',
		'start/install',
		'reference/architecture',
		'connect/mcp',
		'connect/console',
		'connect/ai-guidance',
		'reference/binaries',
	]) {
		const html = await page(`${slug}/index.html`);
		const controls = [
			...html.matchAll(/<docs-copy\b([^>]*)>([\s\S]*?)<\/docs-copy>/g),
		];
		assert.equal(controls.length, 3, slug);
		assert.match(controls[1][1], /data-src="\/llms-full\.txt"/);
		assert.match(controls[2][1], /data-kind="link"/);
		assert.match(html, /<summary[^>]*>Use with AI/);
		assert.match(html, /<a\b[^>]*href="\/llms\.txt"[^>]*>Browse the llms.txt index<\/a>/);
		assert.match(controls[0][1], new RegExp(`data-src="/${slug}\\.md\\.txt"`));
		assert.match(
			controls[0][2],
			/<button\b[^>]*type="button"[^>]*>[\s\S]*?<span[^>]*>Copy page<\/span>/,
		);
		assert.match(
			html,
			new RegExp(
				`<a\\b[^>]*href="/${slug}\\.md\\.txt"[^>]*>View Markdown<\\/a>`,
			),
		);
		assert.match(
			html,
			new RegExp(
				`<link\\b(?=[^>]*rel="alternate")(?=[^>]*type="text/markdown")(?=[^>]*href="/${slug}\\.md\\.txt")[^>]*>`,
			),
		);
		assert.doesNotMatch(html, /class="copy-page-action"/);
	}
});

test('the landing page and every guide expose the site index for discovery', async () => {
	const files = (await readdir(output, { recursive: true })).filter((file) =>
		file.endsWith('index.html'),
	);
	assert.ok(files.length > 20);
	for (const file of files) {
		assert.match(
			await page(file),
			/<link\b(?=[^>]*rel="describedby")(?=[^>]*type="text\/plain")(?=[^>]*href="https:\/\/pds\.linkjar\.io\/llms\.txt")[^>]*>/,
			file,
		);
	}
});

test('focused bundles are discoverable, contain complete guides, and omit unrelated guides', async () => {
	const index = await page('llms.txt');
	const full = await page('llms-full.txt');
	const guide = await page('connect/ai-guidance.md.txt');
	for (const { name, includes, excludes } of [
		{
			name: 'setup',
			includes: ['start/native', 'start/install', 'start/verify', 'configure/environment', 'operate/cli'],
			excludes: ['connect/mcp', 'operate/backup'],
		},
		{
			name: 'operations',
			includes: ['operate/backup', 'operate/update', 'operate/migrate-from-reference', 'reference/troubleshooting', 'scale/postgres'],
			excludes: ['start/native', 'connect/mcp'],
		},
		{
			name: 'clients',
			includes: ['connect/mcp', 'connect/console', 'reference/binaries', 'scale/sidecars'],
			excludes: ['start/native', 'operate/backup'],
		},
	]) {
		const file = `llms-${name}.txt`;
		const bundle = await page(file);
		assert.ok(index.includes(`https://pds.linkjar.io/${file}`), `${file} is missing from the index`);
		assert.ok(guide.includes(`https://pds.linkjar.io/${file}`), `${file} is missing from the guide`);
		assert.ok(bundle.length < full.length, `${file} must be smaller than the complete export`);
		for (const slug of includes) {
			assert.ok(bundle.includes((await page(`${slug}.md.txt`)).trim()), `${file}: missing ${slug}`);
		}
		for (const slug of excludes) {
			assert.ok(!bundle.includes(`[Source](https://pds.linkjar.io/${slug}/)`), `${file}: unexpected ${slug}`);
		}
		assert.doesNotMatch(bundle, /\]\((?:\/|#)/, `${file}: links must be portable`);
	}
});

test('native setup exports preserve language labels and configuration text without UI markup', async () => {
	const markdown = await page('start/native.md.txt');
	for (const language of ['bash', 'nix', 'dotenv', 'ini', 'caddyfile']) {
		assert.ok(markdown.includes(`\`\`\`${language}\n`), `missing ${language} fence`);
	}
	assert.match(markdown, /reverse_proxy 127\.0\.0\.1:2583/);
	assert.match(markdown, /PDS_HOSTNAME=pds\.example\.com/);
	assert.doesNotMatch(markdown, /<span|--0:|Copy to clipboard|data-language=/);
});

test('rendered Astro text stays readable and utility controls stay out of the copied content', async () => {
	const overview = await page('start/overview.md.txt');
	assert.match(
		overview,
		/\[Set up a new server\]\(https:\/\/pds\.linkjar\.io\/start\/requirements\/\)\s+Prepare a host/,
	);
	const landing = await page('index.md.txt');
	assert.match(landing, /Personal Data Server written in Rust/);
	assert.match(landing, /SQLite/);
	assert.match(landing, /S3-compatible blob storage or Postgres/);
	assert.match(landing, /\[Source\]\(https:\/\/pds\.linkjar\.io\/\)/);
	assert.doesNotMatch(
		landing,
		/Copy all docs|Bring the docs|Already up and running|Made by LinkJar|<LandingContent\b|^import /m,
	);
	assert.doesNotMatch(landing, /\[(?:\d|↗)\]\(/);
	assert.match(
		landing,
		/\[Prepare a host and domain\]\(https:\/\/pds\.linkjar\.io\/start\/requirements\/\)/,
	);
});

test('every published Markdown page retains its canonical source in the index and full export', async () => {
	const files = (await readdir(output, { recursive: true })).filter((file) =>
		file.endsWith('.md.txt'),
	);
	const index = await page('llms.txt');
	const full = await page('llms-full.txt');
	assert.ok(files.length > 20, 'expected the complete documentation export');
	for (const file of files) {
		const markdown = await page(file);
		const path = file === 'index.md.txt' ? '' : file.replace(/\.md\.txt$/, '/');
		assert.ok(
			markdown.includes(`[Source](https://pds.linkjar.io/${path})`),
			file,
		);
		assert.ok(index.includes(`https://pds.linkjar.io/${file}`), file);
		assert.ok(
			full.includes(markdown.trim()),
			`${file} differs from the combined export`,
		);
		assert.doesNotMatch(
			markdown,
			/<\/?(?:Tabs|TabItem|Steps|FileTree|Diagram|LandingContent)\b|^import /m,
			file,
		);
	}
	assert.ok(
		!files.includes('404.md.txt'),
		'the error page must not become documentation',
	);
	const landing = await page('index.html');
	assert.equal((landing.match(/<docs-copy\b/g) ?? []).length, 1);
	assert.match(landing, /<docs-copy\b[^>]*data-src="\/llms-full\.txt"/);
	assert.doesNotMatch(
		landing,
		/<a\b[^>]*href="https:\/\/(?:chatgpt\.com|claude\.ai)/,
	);
});
