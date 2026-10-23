// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightThemeBlack from 'starlight-theme-black';
import llmDocs from './src/integrations/llm-docs.mjs';

// The self-hosting guide of LinkJar PDS. Pages are Markdown under
// src/content/docs/; the architecture diagrams come from
// ../docs/architecture/svg through src/components/Diagram.astro.
export default defineConfig({
	site: 'https://pds.linkjar.io',
	integrations: [
		starlight({
			plugins: [
				starlightThemeBlack({
					docs: { showMarkdownActions: false },
					navLinks: [
						{ label: 'Home', link: '/' },
						{ label: 'Docs', link: '/start/overview/' },
						{ label: 'GitHub', link: 'https://github.com/linkjar/pds' },
					],
				}),
				llmDocs(),
			],
			title: 'LinkJar PDS',
			description:
				'Install, configure, and operate LinkJar PDS, an AT Protocol Personal Data Server written in Rust.',
			logo: { src: './src/assets/linkjar-256.webp', alt: '' },
			favicon: '/favicon.ico',
			routeMiddleware: './src/route-data.ts',
			head: [
				{ tag: 'link', attrs: { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/logo/linkjar-32.png' } },
				{ tag: 'link', attrs: { rel: 'apple-touch-icon', sizes: '180x180', href: '/logo/linkjar-180.png' } },
				{ tag: 'meta', attrs: { property: 'og:locale', content: 'en_US' } },
				{ tag: 'meta', attrs: { property: 'og:image', content: 'https://pds.linkjar.io/linkjar-pds-og-v1.jpg' } },
				{ tag: 'meta', attrs: { property: 'og:image:type', content: 'image/jpeg' } },
				{ tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
				{ tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
				{ tag: 'meta', attrs: { property: 'og:image:alt', content: 'LinkJar PDS jar logo beside the words: Your data. Your server. The open network. An open-source AT Protocol Personal Data Server.' } },
			],
			editLink: { baseUrl: 'https://github.com/linkjar/pds/edit/main/site/' },
			lastUpdated: true,
			tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
			components: {
				Hero: './src/components/LandingHero.astro',
				PageTitle: './src/components/DocsPageTitle.astro',
				SiteTitle: './src/components/ThemeSiteTitle.astro',
			},
			customCss: ['./src/styles/custom.css'],
			expressiveCode: { frames: { showCopyToClipboardButton: true } },
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Overview', slug: 'start/overview' },
						{ label: 'Requirements', slug: 'start/requirements' },
						{ label: 'Builds and downloads', slug: 'start/builds' },
						{ label: 'Binaries and ports', slug: 'reference/binaries' },
					],
				},
				{
					label: 'Install a server',
					items: [
						{ label: 'DNS and domains', slug: 'start/dns' },
						{ label: 'Nix or a native binary', slug: 'start/native' },
						{ label: 'Docker Compose', slug: 'start/install' },
						{ label: 'Verify the setup', slug: 'start/verify' },
						{ label: 'Create an account', slug: 'start/first-account' },
						{ label: 'Migrate an existing PDS', slug: 'operate/migrate-from-reference' },
					],
				},
				{
					label: 'Connect clients',
					items: [
						{ label: 'AI clients with MCP', slug: 'connect/mcp' },
						{ label: 'Operations console', slug: 'connect/console' },
						{ label: 'Use these docs with AI', slug: 'connect/ai-guidance' },
					],
				},
				{
					label: 'Configure',
					collapsed: true,
					items: [
						{ label: 'Environment variables', slug: 'configure/environment' },
						{ label: 'Profiles', slug: 'configure/profiles' },
						{ label: 'Email', slug: 'configure/email' },
						{ label: 'Blob storage', slug: 'configure/blob-storage' },
						{ label: 'Custom handles', slug: 'configure/handles' },
					],
				},
				{
					label: 'Operate',
					collapsed: true,
					items: [
						{ label: 'Accounts', slug: 'operate/accounts' },
						{ label: 'Back up and restore', slug: 'operate/backup' },
						{ label: 'Update and roll back', slug: 'operate/update' },
						{ label: 'Monitoring', slug: 'operate/monitoring' },
						{ label: 'Audit log', slug: 'operate/audit-log' },
						{ label: 'Operator commands', slug: 'operate/cli' },
					],
				},
				{
					label: 'Scale',
					collapsed: true,
					items: [
						{ label: 'Postgres', slug: 'scale/postgres' },
						{ label: 'Entryway and members', slug: 'scale/entryway' },
						{ label: 'Sidecars with Compose', slug: 'scale/sidecars' },
					],
				},
				{
					label: 'Reference',
					collapsed: true,
					items: [
						{ label: 'Architecture', slug: 'reference/architecture' },
						{ label: 'Troubleshooting', slug: 'reference/troubleshooting' },
					],
				},
			],
		}),
	],
	vite: {
		// Diagram.astro imports the rendered SVGs of ../docs/architecture.
		server: { fs: { allow: ['..'] } },
	},
});
