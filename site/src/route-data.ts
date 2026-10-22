import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

// Use the final Open Graph values, including any page-specific overrides.
export const onRequest = defineRouteMiddleware(({ locals }) => {
	const { head } = locals.starlightRoute;
	for (const field of ['title', 'description', 'image', 'image:alt']) {
		if (head.some(({ attrs }) => attrs?.name === `twitter:${field}`)) continue;
		const content = head.find(
			({ tag, attrs }) => tag === 'meta' && attrs?.property === `og:${field}`,
		)?.attrs?.content;
		if (content) {
			head.push({ tag: 'meta', attrs: { name: `twitter:${field}`, content } });
		}
	}
});
