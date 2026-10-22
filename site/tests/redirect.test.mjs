import assert from 'node:assert/strict';
import test from 'node:test';
import worker from '../redirect/worker.js';

test('redirects HTTP to HTTPS without changing an encoded path or repeated query', async () => {
	const response = await worker.fetch(new Request(
		'http://pds.linkjar.io/start/a%2Fb%20c/?tag=one&tag=two&encoded=%2F%3F&space=a+b',
	));
	assert.equal(response.status, 308);
	assert.equal(
		response.headers.get('Location'),
		'https://pds.linkjar.io/start/a%2Fb%20c/?tag=one&tag=two&encoded=%2F%3F&space=a+b',
	);
	assert.equal(await response.text(), '');
});

test('rejects requests outside the HTTP docs origin without redirecting', async () => {
	for (const url of [
		'https://pds.linkjar.io/start/install/',
		'http://linkjar.io/',
		'http://other.pds.linkjar.io/',
		'http://pds.linkjar.io.evil.example/',
		'http://pds.linkjar.io:8080/',
	]) {
		const response = await worker.fetch(new Request(url));
		assert.equal(response.status, 404, url);
		assert.equal(response.headers.get('Location'), null, url);
	}
});

test('keeps the fixed docs origin when paths, queries, or headers name another host', async () => {
	const response = await worker.fetch(new Request(
		'http://pds.linkjar.io//evil.example/path?next=https%3A%2F%2Fevil.example',
		{ headers: { Host: 'evil.example', 'X-Forwarded-Host': 'evil.example' } },
	));
	assert.equal(response.status, 308);
	assert.equal(
		response.headers.get('Location'),
		'https://pds.linkjar.io//evil.example/path?next=https%3A%2F%2Fevil.example',
	);
});

test('redirects the root and preserves an empty query without adding a body', async () => {
	for (const [url, location] of [
		['http://pds.linkjar.io/', 'https://pds.linkjar.io/'],
		['http://pds.linkjar.io/start/install/?', 'https://pds.linkjar.io/start/install/?'],
	]) {
		const response = await worker.fetch(new Request(url, { method: 'HEAD' }));
		assert.equal(response.status, 308);
		assert.equal(response.headers.get('Location'), location);
		assert.equal(await response.text(), '');
	}
});
