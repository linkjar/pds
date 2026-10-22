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
			promote: ['index', 'start/overview', 'start/**'],
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
