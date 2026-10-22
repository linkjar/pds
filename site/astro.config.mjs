// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// The self-hosting guide of LinkJar PDS. Pages are Markdown under
// src/content/docs/; the architecture diagrams come from
// ../docs/architecture/svg through src/components/Diagram.astro.
export default defineConfig({
	site: 'https://pds.linkjar.io',
	integrations: [
		starlight({
			title: 'LinkJar PDS',
			description:
				'Install, configure, and operate LinkJar PDS, an AT Protocol Personal Data Server written in Rust.',
			logo: { src: './src/assets/linkjar-256.webp', alt: '' },
			favicon: '/favicon.ico',
			head: [
				{ tag: 'link', attrs: { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/logo/linkjar-32.png' } },
				{ tag: 'link', attrs: { rel: 'apple-touch-icon', sizes: '180x180', href: '/logo/linkjar-180.png' } },
			],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/linkjar/pds' }],
			editLink: { baseUrl: 'https://github.com/linkjar/pds/edit/main/site/' },
			lastUpdated: true,
			tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
			customCss: ['./src/styles/custom.css'],
			sidebar: [
				{
					label: 'Get started',
					items: [
						{ label: 'Overview', slug: '' },
						{ label: 'Requirements', slug: 'start/requirements' },
						{ label: 'DNS and domains', slug: 'start/dns' },
						{ label: 'Install with Docker', slug: 'start/install' },
						{ label: 'Verify the setup', slug: 'start/verify' },
						{ label: 'Create an account', slug: 'start/first-account' },
					],
				},
				{ label: 'Configure', items: [{ autogenerate: { directory: 'configure' } }] },
				{ label: 'Operate', items: [{ autogenerate: { directory: 'operate' } }] },
				{
					label: 'Scale',
					collapsed: true,
					items: [{ autogenerate: { directory: 'scale' } }],
				},
				{ label: 'Reference', items: [{ autogenerate: { directory: 'reference' } }] },
			],
		}),
	],
	vite: {
		// Diagram.astro imports the rendered SVGs of ../docs/architecture.
		server: { fs: { allow: ['..'] } },
	},
});
