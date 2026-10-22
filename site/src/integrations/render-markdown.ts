import type { APIContext } from 'astro';
import type { CollectionEntry } from 'astro:content';
import renderSimpleMarkdown from 'starlight-llm-actions/internal/simple-markdown';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkStringify from 'remark-stringify';
import { visit } from 'unist-util-visit';

// Render MDX through the plugin first, then keep copied links portable and
// restore the text boundary lost when a LinkCard's inline spans are flattened.
export default async function renderMarkdown(
	entry: CollectionEntry<'docs'>,
	context: APIContext,
) {
	if (!context.site)
		throw new Error('Markdown exports require an Astro site URL.');
	const slug =
		entry.id === 'index'
			? ''
			: `${entry.id.split('/').map(encodeURIComponent).join('/')}/`;
	const source = new URL(
		`${import.meta.env.BASE_URL.replace(/\/$/, '')}/${slug}`,
		context.site,
	);
	const rendered = await renderSimpleMarkdown(entry, context);
	const markdown = unified()
		.use(remarkParse)
		.use(remarkGfm)
		.use(remarkStringify);
	const tree = markdown.parse(rendered);
	visit(tree, ['link', 'image', 'definition'], (node) => {
		if ('url' in node && typeof node.url === 'string')
			node.url = new URL(node.url, source).href;
	});
	visit(tree, 'paragraph', (node) => {
		const [title, description] = node.children;
		if (
			node.children.length === 2 &&
			title.type === 'link' &&
			description.type === 'text' &&
			/^[A-Z]/.test(description.value)
		) {
			description.value = ` ${description.value}`;
		}
	});
	return `[Source](${source.href})\n\n${markdown.stringify(tree).trim()}`;
}
