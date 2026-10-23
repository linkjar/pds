// @ts-check
import starlightLlmActions from 'starlight-llm-actions';

// One renderer supplies both individual pages and the complete documentation.
// Black provides the visual theme; our copy controls fetch these public files.
export default function llmDocs() {
	return starlightLlmActions({
		renderMarkdown: { module: './src/integrations/render-markdown.ts' },
		markdownUrl: '/{slug}.md.txt',
		linkAlternate: true,
		llmsTxt: {
			title: 'LinkJar PDS',
			description:
				'Official guides for installing, configuring, and operating LinkJar PDS, an AT Protocol Personal Data Server written in Rust. Start with the overview, then choose NixOS, a native binary, or Docker Compose. MCP and the operations console are optional services.',
			promote: [
				'index', 'start/overview', 'start/requirements', 'start/builds',
				'reference/binaries', 'start/dns', 'start/native', 'start/install',
				'start/verify', 'start/first-account',
			],
			subsets: [
				{
					label: 'Setup',
					description: 'Requirements, builds, installation, configuration, and verification.',
					paths: ['start/**', 'configure/**', 'reference/binaries', 'operate/cli'],
				},
				{
					label: 'Operations',
					description: 'Accounts, backups, updates, migration, monitoring, scaling, and troubleshooting.',
					paths: ['operate/**', 'scale/**', 'configure/environment', 'reference/troubleshooting', 'reference/architecture'],
				},
				{
					label: 'Clients',
					description: 'MCP, OAuth, the operations console, sidecar builds, and network ports.',
					paths: ['connect/**', 'reference/binaries', 'scale/sidecars'],
				},
			],
		},
		actions: {
			copyMarkdown: false,
			viewMarkdown: false,
			printPdf: false,
			openIn: false,
		},
		printNotice: false,
	});
}
