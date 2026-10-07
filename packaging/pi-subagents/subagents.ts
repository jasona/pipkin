import { type Context, defineService, type ReplicatedState } from "@earendil-works/chord";
import type { ConversationView, TaskGraph } from "@earendil-works/pi-durable";
import type { HistoryPage, HistoryPageRequest } from "./history.ts";

/** A durable child owned by a subagent tool call in this session's root conversation. */
export interface SubagentSummary {
	conversationId: number;
	taskId: number;
	callId: string;
	task: string;
	status: "pending" | "running" | "waiting" | "completing" | "done" | "failed" | "aborted";
}

export interface SubagentTranscript {
	readonly state: ReplicatedState<ConversationView>;
}

/** Keyed child views are created lazily by Subagents.open() and stream Pi's durable view. */
export const SubagentTranscript = defineService<SubagentTranscript>("pi.subagent-transcript");

export interface Subagents {
	/** Pi owns the live task graph; terminal children must be read from list(), not this state. */
	readonly activity: ReplicatedState<TaskGraph>;
	readonly configuration: ReplicatedState<{ enabled: boolean }>;
	list(context: Context): Promise<SubagentSummary[]>;
	view(conversationId: number, context: Context): Promise<ConversationView>;
	/** Mount a live pi.subagent-transcript keyed by the child id; idempotent within the Session. */
	open(conversationId: number, context: Context): Promise<void>;
	page(conversationId: number, request: HistoryPageRequest, context: Context): Promise<HistoryPage>;
	setEnabled(enabled: boolean, context: Context): Promise<void>;
}

export const Subagents = defineService<Subagents>("pi.subagents");
