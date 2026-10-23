// The Caddyfile syntax used in our proxy examples. No runtime parser is shipped.
export default {
	name: 'caddyfile',
	scopeName: 'source.caddyfile',
	patterns: [
		{ name: 'comment.line.number-sign.caddyfile', match: '(?:^|\\s)#.*$' },
		{
			name: 'string.quoted.double.caddyfile',
			begin: '"',
			end: '"',
			patterns: [{ name: 'constant.character.escape.caddyfile', match: '\\\\.' }],
		},
		{ name: 'string.quoted.raw.caddyfile', begin: '`', end: '`' },
		{ name: 'variable.other.caddyfile', match: '\\{(?:\\$|[a-zA-Z_])[^}\\s]*\\}' },
		{
			match: '^\\s*([a-z_][a-z0-9_]*)(?=\\s|$)',
			captures: { 1: { name: 'keyword.other.caddyfile' } },
		},
		{
			name: 'string.unquoted.caddyfile',
			match: '(?:https?://)?(?:[a-zA-Z0-9*_-]+\\.)+[a-zA-Z0-9_-]+(?::[0-9]+)?(?:/[^\\s{}]*)?',
		},
		{ name: 'constant.numeric.caddyfile', match: '\\b[0-9]+(?:ms|s|m|h)?\\b' },
		{ name: 'punctuation.section.block.caddyfile', match: '[{}]' },
	],
};
