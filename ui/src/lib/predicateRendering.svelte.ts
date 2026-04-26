// Predicate-rendering registry. The notebook view consults this map to
// pick a renderer for each predicate. Anything not listed falls through
// to the generic mono key/value row.
//
// To add a new known predicate: add one line. No other changes needed.

export type RendererKind =
	| 'heading' // primary entity name — large display text
	| 'tag' // small chip beside the heading
	| 'lead' // serif lead paragraph (entity summary, claim text)
	| 'doc-link' // external documentation URL
	| 'relation' // value points at another entity / fact id
	| 'status-tag' // status chip (feature/status, …)
	| 'kv'; // generic mono key/value row (fallback)

export const PREDICATE_RENDERERS: Record<string, RendererKind> = {
	'entity/name': 'heading',
	'entity/type': 'tag',
	'concept/summary': 'lead',
	'concept/docs_url': 'doc-link',
	'concept/scalar_docs_url': 'doc-link',
	operates_on: 'relation',
	input_to: 'relation',
	lowers_to: 'relation',
	'feature/status': 'status-tag',
	'lecture/topic': 'tag',
	'exom/summary': 'lead',
	'exom/description': 'lead'
};

export function rendererFor(predicate: string): RendererKind {
	return PREDICATE_RENDERERS[predicate] ?? 'kv';
}

// Entities are derived from fact ids. The convention is `<entity>#<aspect>`,
// e.g. `concept/verb#name` belongs to entity `concept/verb`. Fact ids
// without a `#` form their own single-aspect entity.
export function entityForFactId(factId: string): string {
	const idx = factId.indexOf('#');
	return idx === -1 ? factId : factId.slice(0, idx);
}
