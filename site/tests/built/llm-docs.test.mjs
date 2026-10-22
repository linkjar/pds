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

test('documentation pages render one working copy control and a link to that page’s Markdown', async () => {
	for (const slug of [
		'start/overview',
		'start/install',
		'reference/architecture',
	]) {
		const html = await page(`${slug}/index.html`);
		const controls = [
			...html.matchAll(/<docs-copy\b([^>]*)>([\s\S]*?)<\/docs-copy>/g),
		];
		assert.equal(controls.length, 1, slug);
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
