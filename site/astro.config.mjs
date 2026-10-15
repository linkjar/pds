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
				'Host your own AT Protocol Personal Data Server: a small Rust server, compatible with the reference PDS.',
			logo: { src: './src/assets/logo.svg', alt: 'LinkJar PDS' },
			favicon: '/favicon.svg',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/linkjar/pds' }],
			editLink: { baseUrl: 'https://github.com/linkjar/pds/edit/main/site/' },
			lastUpdated: true,
			customCss: [
				'@fontsource/ibm-plex-sans/400.css',
				'@fontsource/ibm-plex-sans/500.css',
				'@fontsource/ibm-plex-sans/600.css',
				'@fontsource/ibm-plex-mono/400.css',
				'@fontsource/ibm-plex-mono/500.css',
				'./src/styles/custom.css',
			],
			sidebar: [
				{ label: 'Get started', items: [{ autogenerate: { directory: 'start' } }] },
				{ label: 'Configure', items: [{ autogenerate: { directory: 'configure' } }] },
				{ label: 'Operate', items: [{ autogenerate: { directory: 'operate' } }] },
				{ label: 'Scale', items: [{ autogenerate: { directory: 'scale' } }] },
				{ label: 'Reference', items: [{ autogenerate: { directory: 'reference' } }] },
			],
		}),
	],
	vite: {
		// Diagram.astro imports the rendered SVGs of ../docs/architecture.
		server: { fs: { allow: ['..'] } },
	},
});
