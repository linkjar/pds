import assert from 'node:assert/strict';
import test from 'node:test';
import { copyDocument } from '../src/lib/copy-document.mjs';

const origin = 'https://pds.linkjar.io';
const signal = new AbortController().signal;

test('copy starts during user activation and waits for the fetched Markdown', async () => {
	let resolveResponse;
	const response = new Promise((resolve) => { resolveResponse = resolve; });
	let started = false;
	let copied;
	class Item { constructor(content) { this.content = content; } }
	const copying = copyDocument('/connect/mcp.md.txt', {
		origin, signal, item: Item,
		fetcher: async (url, options) => {
			assert.equal(url.href, `${origin}/connect/mcp.md.txt`);
			assert.equal(options.credentials, 'omit');
			assert.equal(options.redirect, 'error');
			assert.equal(options.mode, 'same-origin');
			assert.equal(options.signal, signal);
			return response;
		},
		clipboard: {
			write: async ([item]) => {
				started = true;
				copied = await (await item.content['text/plain']).text();
			},
		},
	});
	assert.equal(started, true, 'write must start before the network finishes');
	resolveResponse(new Response('# MCP\n', { headers: { 'Content-Type': 'text/plain; charset=utf-8' } }));
	await copying;
	assert.equal(copied, '# MCP\n');
});

test('cross-origin and empty sources never make a request', async () => {
	for (const source of ['', 'https://elsewhere.example/docs', '//elsewhere.example/docs']) {
		await assert.rejects(copyDocument(source, {
			origin, signal, clipboard: {},
			fetcher: () => assert.fail('must not fetch an untrusted source'),
		}), /Expected a document/);
	}
});

test('error pages and HTML responses never replace the clipboard', async () => {
	for (const response of [
		new Response('not found', { status: 404 }),
		new Response('<html>error</html>', { headers: { 'Content-Type': 'text/html' } }),
	]) {
		await assert.rejects(copyDocument('/guide.md.txt', {
			origin, signal, fetcher: async () => response,
			clipboard: { writeText: () => assert.fail('must not copy an error page') },
		}), /could not be loaded/);
	}
});

test('plain-text clipboard fallback preserves the document', async () => {
	let copied;
	await copyDocument('/guide.md.txt', {
		origin, signal,
		fetcher: async () => new Response('# Guide\n', { headers: { 'Content-Type': 'text/markdown' } }),
		clipboard: { writeText: async (text) => { copied = text; } },
	});
	assert.equal(copied, '# Guide\n');
});

test('denial is reported even when the pending fetch subsequently fails', async () => {
	class Item { constructor(content) { this.content = content; } }
	await assert.rejects(copyDocument('/guide.md.txt', {
		origin, signal, item: Item,
		fetcher: async () => { throw new Error('offline'); },
		clipboard: { write: async () => { throw new Error('clipboard denied'); } },
	}), /clipboard denied/);
});
