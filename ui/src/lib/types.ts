export interface ExomEntry {
	name: string;
	description: string;
	created_at: number;
	updated_at: number;
	archived: boolean;
	archived_at?: number | null;
}

export interface ExomemStatus {
	ok: boolean;
	exom: string;
	/** Active branch id for this exom (from server brain). */
	current_branch?: string;
	server: {
		name: string;
		version: string;
		uptime_sec: number;
	};
	storage: {
		exom_path: string;
	};
	stats: {
		relations: number;
		facts: number;
		derived_tuples: number;
		intervals: number;
		directives: number;
		events_logged: number;
	};
}

export interface ExomemSchemaRelation {
	name: string;
	arity: number;
	kind: 'base' | 'derived';
	cardinality: number;
	has_intervals: boolean;
	defined_by: string[];
	sample_tuples?: unknown[][];
}

export interface ExomemSchemaResponse {
	relations: ExomemSchemaRelation[];
	directives: Array<{
		type: string;
		predicate: string;
	}>;
	summary: {
		relation_count: number;
		base_relation_count: number;
		derived_relation_count: number;
		largest_relation: {
			name: string;
			cardinality: number;
		} | null;
	};
}

export interface ExomemClusterSummary {
	id: string;
	label: string;
	kind: 'shared_subject' | 'shared_predicate' | 'shared_object' | string;
	fact_count: number;
	active_count: number;
	deprecated_count: number;
}

export interface ExomemGraphNode {
	id: string;
	type: 'entity' | 'fact';
	label: string;
	degree?: number;
	cluster_ids?: string[];
	meta?: Record<string, unknown>;
}

export interface ExomemGraphEdge {
	id: string;
	type: string;
	source: string;
	target: string;
	label: string;
	cluster_ids?: string[];
	meta?: Record<string, unknown>;
}

export interface ExomemGraphResponse {
	nodes: ExomemGraphNode[];
	edges: ExomemGraphEdge[];
	clusters: ExomemClusterSummary[];
	summary: {
		node_count: number;
		edge_count: number;
		cluster_count: number;
	};
}

export interface ExomemLoggedEvent {
	id: string;
	type: 'query' | 'assert' | 'retract' | 'evaluate' | 'load' | string;
	timestamp: string;
	query_text?: string;
	relations_scanned?: string[];
	tuples_matched?: number;
	predicate?: string;
	terms?: string[];
	new_derivations?: number;
	duration_ms?: number;
	pattern?: string;
	tuples_retracted?: number;
	mode?: string;
	source?: string;
	facts_added?: number;
	rules_added?: number;
}

// ---------------------------------------------------------------------------
// Memory states
// ---------------------------------------------------------------------------

export type MemoryState =
	| 'active'
	| 'draft'
	| 'derived'
	| 'deprecated'
	| 'disabled'
	| 'retracted'
	| 'conflict'
	| 'stale'
	| 'historical'
	| 'future';

export const MEMORY_STATE_COLORS: Record<MemoryState, string> = {
	active: 'text-fact-base',
	draft: 'text-muted-foreground',
	derived: 'text-fact-derived',
	deprecated: 'text-rule-accent',
	disabled: 'text-muted-foreground/60',
	retracted: 'text-contra',
	conflict: 'text-contra',
	stale: 'text-rule-accent',
	historical: 'text-muted-foreground',
	future: 'text-fact-derived',
};

export const MEMORY_STATE_BG: Record<MemoryState, string> = {
	active: 'bg-fact-base/10 border-fact-base/30',
	draft: 'bg-muted/30 border-muted-foreground/30',
	derived: 'bg-fact-derived/10 border-fact-derived/30',
	deprecated: 'bg-rule-accent/10 border-rule-accent/30',
	disabled: 'bg-muted/20 border-muted-foreground/20',
	retracted: 'bg-contra/10 border-contra/30',
	conflict: 'bg-contra/10 border-contra/30',
	stale: 'bg-rule-accent/10 border-rule-accent/30',
	historical: 'bg-muted/20 border-muted-foreground/20',
	future: 'bg-fact-derived/10 border-fact-derived/30',
};

// ---------------------------------------------------------------------------
// Fact-level types for the Facts CRUD page
// ---------------------------------------------------------------------------

export interface FactEntry {
	predicate: string;
	terms: string[];
	confidence?: number | null;
	source?: string | null;
	kind: 'base' | 'derived';
	status?: MemoryState;
	/** When this fact became true in the real world (ISO 8601). */
	validFrom?: string | null;
	/** When this fact ceased being true (ISO 8601). Null = still valid. */
	validTo?: string | null;
	/** Branch where the fact was asserted (`local` / `inherited` / `override`). */
	branchRole?: 'local' | 'inherited' | 'override' | null;
	/** Branch id that created this version of the fact. */
	branchOrigin?: string | null;
}

// ---------------------------------------------------------------------------
// Rule types for the Rules page
// ---------------------------------------------------------------------------

export interface RuleEntry {
	index: number;
	raw: string;
	head_predicate: string;
	body_atoms: string[];
	uses_negation: boolean;
	uses_temporal: boolean;
	status?: 'active' | 'draft' | 'deprecated' | 'disabled';
	description?: string;
}

// ---------------------------------------------------------------------------
// Query result types
// ---------------------------------------------------------------------------

export interface QueryResult {
	columns: string[];
	rows: unknown[][];
	tuples_matched: number;
	duration_ms: number;
}

// ---------------------------------------------------------------------------
// Entity types
// ---------------------------------------------------------------------------

export interface EntitySummary {
	id: string;
	label: string;
	degree: number;
	predicates: string[];
	factCount: number;
	derivedFactCount: number;
	hasConflicts: boolean;
	hasIntervals: boolean;
}

// ---------------------------------------------------------------------------
// Conflict types
// ---------------------------------------------------------------------------

export interface ConflictEntry {
	id: string;
	type: 'overlapping_interval' | 'contradictory_facts' | 'duplicate' | 'stale';
	description: string;
	facts: FactEntry[];
	predicates: string[];
	severity: 'high' | 'medium' | 'low';
	resolved: boolean;
	resolvedAt?: string;
	resolvedBy?: string;
}

// ---------------------------------------------------------------------------
// Cluster detail (from /api/clusters/:id)
// ---------------------------------------------------------------------------

export interface ClusterDetail {
	id: string;
	label: string;
	kind: string;
	stats: {
		fact_count: number;
		active_count: number;
		deprecated_count: number;
	};
	nodes: Array<{ id: string; type: string; label: string }>;
	facts: Array<{
		id: string;
		tuple: unknown[];
		status: string;
		interval?: { start: string; end: string } | null;
	}>;
	related_clusters: Array<{ id: string; label: string; kind: string }>;
}

// ---------------------------------------------------------------------------
// Fact detail (from /api/facts/:id)
// ---------------------------------------------------------------------------

export interface FactDetail {
	fact: {
		id: string;
		predicate: string;
		tuple: unknown[];
		interval?: { start: string; end: string } | null;
		status: string;
		cluster_ids: string[];
	};
	provenance: { type: 'base' | 'derived' };
	touch_history: Array<{ event_id: string; event_type: string }>;
}

// ---------------------------------------------------------------------------
// Search result types
// ---------------------------------------------------------------------------

export type SearchResultKind = 'fact' | 'rule' | 'entity' | 'cluster' | 'exom';

export interface SearchResult {
	kind: SearchResultKind;
	label: string;
	sublabel: string;
	href?: string;
	data?: unknown;
}
