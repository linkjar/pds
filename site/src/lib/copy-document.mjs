/**
 * Copy a public document without sending credentials or following redirects.
 * Starting the clipboard write before the fetch finishes preserves Safari's
 * user activation. Dependencies are explicit so the failure paths can be tested.
 * @param {string} source
 * @param {{ origin: string, clipboard: Pick<Clipboard, 'write' | 'writeText'>,
 *   item?: typeof ClipboardItem, fetcher: typeof fetch, signal: AbortSignal }} options
 */
export async function copyDocument(source, { origin, clipboard, item, fetcher, signal }) {
	const url = new URL(source, origin);
	if (!source || url.origin !== origin) throw new Error('Expected a document on this site.');
	const text = fetcher(url, {
		credentials: 'omit',
		mode: 'same-origin',
		redirect: 'error',
		signal,
	}).then(async (response) => {
		const type = response.headers.get('Content-Type') ?? '';
		if (!response.ok || !/^text\/(?:plain|markdown)\b/i.test(type)) {
			throw new Error('The documentation could not be loaded.');
		}
		return response.text();
	});
	if (item && clipboard.write) {
		const blob = text.then((value) => new Blob([value], { type: 'text/plain' }));
		// A denied clipboard write may never consume its pending Blob.
		void blob.catch(() => {});
		await clipboard.write([new item({ 'text/plain': blob })]);
	} else {
		await clipboard.writeText(await text);
	}
}
