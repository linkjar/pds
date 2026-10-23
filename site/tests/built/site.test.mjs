import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { join, relative, sep } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import rehypeParse from 'rehype-parse';
import { unified } from 'unified';
import { visit } from 'unist-util-visit';

const output = fileURLToPath(new URL('../../dist/', import.meta.url));
const origin = 'https://pds.linkjar.io';
const parser = unified().use(rehypeParse);
const files = new Set(
	(await readdir(output, { recursive: true, withFileTypes: true }))
		.filter((entry) => entry.isFile())
		.map((entry) =>
			relative(output, join(entry.parentPath, entry.name)).split(sep).join('/'),
		),
);
const pages = new Map(
	await Promise.all(
		[...files]
			.filter((file) => file.endsWith('.html'))
			.map(async (file) => {
				const tree = parser.parse(await readFile(join(output, file), 'utf8'));
				const ids = new Set();
				const links = [];
				let headings = 0;
				visit(tree, 'element', (node) => {
					if (node.properties.id) ids.add(String(node.properties.id));
					if (node.tagName === 'h1') headings++;
					if (
						node.tagName === 'a' &&
						typeof node.properties.href === 'string'
					) {
						links.push(node.properties.href);
					}
				});
				const route = `/${file.endsWith('index.html') ? file.slice(0, -'index.html'.length) : file}`;
				return [file, { ids, links, headings, route }];
			}),
	),
);

test('the landing and existing documentation URLs remain available', () => {
	const routes = [
		'/',
		'/start/overview/',
		'/start/requirements/',
		'/start/dns/',
		'/start/install/',
		'/start/builds/',
		'/start/native/',
		'/start/verify/',
		'/start/first-account/',
		'/configure/blob-storage/',
		'/configure/email/',
		'/configure/environment/',
		'/configure/handles/',
		'/configure/profiles/',
		'/operate/accounts/',
		'/operate/audit-log/',
		'/operate/backup/',
		'/operate/cli/',
		'/operate/migrate-from-reference/',
		'/operate/monitoring/',
		'/operate/update/',
		'/connect/mcp/',
		'/connect/console/',
		'/connect/ai-guidance/',
		'/reference/binaries/',
		'/reference/architecture/',
		'/reference/troubleshooting/',
		'/scale/entryway/',
		'/scale/postgres/',
		'/scale/sidecars/',
	];
	for (const route of routes) {
		assert.ok(
			pages.has(`${route.slice(1)}index.html`),
			`Missing documentation route: ${route}`,
		);
	}
	assert.ok(pages.has('404.html'), 'The static host needs a not-found page.');
});

test('every generated HTML page has one primary heading', () => {
	assert.ok(pages.size > 0, 'Build the site before checking its HTML.');
	for (const [file, page] of pages) {
		assert.equal(page.headings, 1, `${file}: expected one h1`);
	}
});

test('internal links resolve to published files and existing page fragments', () => {
	for (const [file, page] of pages) {
		for (const href of page.links) {
			const url = new URL(href, new URL(page.route, origin));
			if (url.origin !== origin) continue;
			const path = decodeURIComponent(url.pathname.slice(1));
			const target = files.has(path)
				? path
				: `${path.replace(/\/$/, '')}${path ? '/' : ''}index.html`;
			assert.ok(files.has(target), `${file}: missing target for ${href}`);
			if (url.hash && pages.has(target)) {
				const fragment = decodeURIComponent(url.hash.slice(1));
				assert.ok(
					pages.get(target).ids.has(fragment),
					`${file}: missing fragment for ${href}`,
				);
			}
		}
	}
});
