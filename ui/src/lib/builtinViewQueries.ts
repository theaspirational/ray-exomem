import type { ExomemSchemaResponse } from '$lib/types';

export type BuiltinView = NonNullable<ExomemSchemaResponse['ontology']>['builtin_views'][number];

export function varsForArity(arity: number): string[] {
	return Array.from({ length: arity }, (_, i) => `?v${i + 1}`);
}

export function builtinViewQuery(exomPath: string, view: BuiltinView): string {
	const vars = varsForArity(view.arity);
	return `(query ${exomPath} (find ${vars.join(' ')}) (where (${view.name} ${vars.join(' ')})))`;
}
