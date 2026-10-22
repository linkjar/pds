import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import rehypeParse from 'rehype-parse';
import sharp from 'sharp';
import { unified } from 'unified';
import { visit } from 'unist-util-visit';

const output = fileURLToPath(new URL('../../dist/', import.meta.url));
const imageUrl = 'https://pds.linkjar.io/linkjar-pds-og-v1.jpg';

test('published pages have complete, unique sharing tags and page-specific text', async () => {
	const files = (await readdir(output, { recursive: true })).filter((file) =>
		file.endsWith('.html'),
	);
	assert.ok(files.length > 0, 'Build the site before checking sharing tags.');
	const titles = new Set();
	for (const file of files) {
		const tree = unified().use(rehypeParse).parse(await readFile(join(output, file), 'utf8'));
		const meta = new Map();
		let canonical;
		visit(tree, 'element', ({ tagName, properties }) => {
			if (tagName === 'link' && properties.rel?.includes('canonical')) {
				canonical = properties.href;
			}
			if (tagName !== 'meta') return;
			const key = properties.property || properties.name;
			if (typeof key !== 'string' || !/^(og:|twitter:|description$)/.test(key)) return;
			assert.ok(!meta.has(key), `${file}: duplicate ${key}`);
			meta.set(key, properties.content);
		});
		for (const key of ['title', 'description', 'image', 'image:alt']) {
			assert.ok(meta.get(`og:${key}`), `${file}: missing og:${key}`);
			assert.equal(meta.get(`twitter:${key}`), meta.get(`og:${key}`), `${file}: inconsistent ${key}`);
		}
		assert.equal(meta.get('twitter:card'), 'summary_large_image', file);
		assert.equal(meta.get('og:image'), imageUrl, file);
		assert.equal(meta.get('og:image:type'), 'image/jpeg', file);
		assert.equal(meta.get('og:image:width'), '1200', file);
		assert.equal(meta.get('og:image:height'), '630', file);
		assert.equal(meta.get('og:locale'), 'en_US', file);
		assert.equal(meta.get('og:site_name'), 'LinkJar PDS', file);
		assert.equal(meta.get('og:type'), file === 'index.html' ? 'website' : 'article', file);
		assert.equal(meta.get('og:description'), meta.get('description'), file);
		assert.equal(meta.get('og:url'), canonical, file);
		assert.equal(new URL(canonical).origin, 'https://pds.linkjar.io', file);
		assert.ok(!titles.has(meta.get('og:title')), `${file}: repeated page title`);
		titles.add(meta.get('og:title'));
	}
});

test('the advertised social image is published at its declared dimensions', async () => {
	const image = await readFile(join(output, new URL(imageUrl).pathname.slice(1)));
	const metadata = await sharp(image).metadata();
	assert.equal(metadata.format, 'jpeg');
	assert.equal(metadata.width, 1200);
	assert.equal(metadata.height, 630);
	assert.ok(image.length < 300_000, 'Keep the image small for sharing crawlers.');
});
