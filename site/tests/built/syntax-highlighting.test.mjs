import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { select, selectAll } from 'hast-util-select';
import rehypeParse from 'rehype-parse';
import { unified } from 'unified';

const parser = unified().use(rehypeParse);

async function page(slug) {
	const html = await readFile(new URL(`../../dist/${slug}/index.html`, import.meta.url), 'utf8');
	return parser.parse(html);
}

function colors(block, theme) {
	return new Set(selectAll('span[style]', block).flatMap((node) => {
		const color = String(node.properties.style).match(new RegExp(`--${theme}:([^;]+)`));
		return color ? [color[1]] : [];
	}));
}

test('native configuration examples have distinct syntax colors in both themes and copy controls', async () => {
	const tree = await page('start/native');
	for (const language of ['bash', 'nix', 'dotenv', 'ini', 'caddyfile']) {
		const block = select(`pre[data-language="${language}"]`, tree);
		assert.ok(block, `${language}: missing example`);
		for (const theme of [0, 1]) {
			assert.ok(colors(block, theme).size >= 2, `${language}: theme ${theme} renders as plain text`);
		}
	}
	for (const frame of selectAll('.expressive-code .frame', tree)) {
		assert.ok(select('.copy button[data-code]', frame), 'code must remain copyable');
	}
});

test('Caddy directives and addresses have different colors without changing the copied configuration', async () => {
	const tree = await page('connect/mcp');
	const frame = selectAll('.expressive-code .frame', tree).find((node) =>
		select('pre[data-language="caddyfile"]', node),
	);
	assert.ok(frame, 'missing the Caddyfile example');
	const tokens = selectAll('pre span[style]', frame);
	const token = (text) => tokens.find((node) =>
		node.children.some((child) => child.type === 'text' && child.value === text),
	);
	const directive = token('reverse_proxy');
	const address = token('127.0.0.1:2586');
	assert.ok(directive && address, 'both directive and address must be highlighted');
	for (const theme of [0, 1]) {
		assert.notDeepEqual(colors(directive, theme), colors(address, theme));
	}
	const button = select('.copy button[data-code]', frame);
	assert.equal(button.properties.dataCode, 'mcp.example.com {\u007f    reverse_proxy 127.0.0.1:2586\u007f}');
});
