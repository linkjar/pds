export default {
	/**
	 * @param {Request} request
	 * @returns {Response | Promise<Response>}
	 */
	fetch(request) {
		const url = new URL(request.url);
		// Only this origin may reach the HTTP route; never redirect another host.
		if (url.origin !== 'http://pds.linkjar.io') {
			return new Response(null, { status: 404 });
		}
		url.protocol = 'https:';
		return Response.redirect(url.href, 308);
	},
};
