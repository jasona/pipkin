import { type Context, defineFacet, type Facet } from "@earendil-works/chord";
import {
	AgentDoc,
	AssistantEntry,
	type ConversationId,
	type Cursor,
	type EntryId,
	type Harness,
} from "@earendil-works/pi-durable";
import { Subagent } from "../durable/subagent.ts";
import { createHistory } from "./history-provider.ts";
import { type SubagentSummary, Subagents, SubagentTranscript } from "./subagents.ts";

/** A session-local, presentation-safe facade; no child can be addressed by guessing an id in another Session. */
export async function createSubagentsServiceFacet(
	harness: Harness,
	rootId: ConversationId,
	context: Context,
): Promise<Facet> {
	const root = (await harness.conversation(rootId, context))!;
	const activity = await harness.taskGraph(context);
	const agent = await harness.documentState(AgentDoc, rootId, context);
	if (agent === undefined) {
		activity.dispose();
		throw new Error(`Root conversation ${rootId} has no agent document`);
	}
	const selected = async (ctx: Context) => (await root.agent(ctx)).tools.some((tool) => tool.name === "subagent");
	const enabled = await selected(context);

	return defineFacet({
		id: "@pi/subagents",
		setup(env) {
			env.own(() => activity.dispose());
			env.own(() => agent.dispose());
			const configuration = env.replicatedState({ enabled });
			const transcripts = env.provideMany(SubagentTranscript);
			const mounted = new Map<number, Promise<void>>();
			env.onActivate(() => {
				let revision = 0;
				const unsubscribe = agent.subscribe((_value, ctx) => {
					const current = ++revision;
					void selected(ctx).then(
						(next) => {
							if (revision !== current) return;
							configuration.change(ctx, (draft) => {
								draft.enabled = next;
							});
						},
						(error: unknown) => console.error("Subagent configuration sync failed", error),
					);
				});
				env.own(() => {
					revision++;
					unsubscribe();
				});
			});
			const list = async (ctx: Context): Promise<SubagentSummary[]> => {
				const children: SubagentSummary[] = [];
				let cursor: Cursor | undefined;
				do {
					const page = await harness.commit(
						(tx) => tx.scanConversations({ ownerConversationId: rootId }, 128, cursor),
						ctx,
					);
					for (const record of page.items) {
						if (record.owner?.conversationId !== rootId) continue;
						const task = await harness.commit((tx) => tx.task(record.owner!.taskId), ctx);
						if (task?.kind !== "pi.tool") continue;
						const input = task.input;
						if (input === null || typeof input !== "object" || Array.isArray(input)) continue;
						const entryId = input.assistant;
						const callId = input.callId;
						if (!Number.isSafeInteger(entryId) || typeof callId !== "string") continue;
						const entry = await harness.commit((tx) => tx.entry(AssistantEntry, entryId as EntryId), ctx);
						const call =
							entry?.model?.[0]?.role === "assistant"
								? entry.model[0].content.find((part) => part.type === "toolCall" && part.id === callId)
								: undefined;
						if (call?.type !== "toolCall" || call.name !== "subagent") continue;
						const taskText = (call.arguments as { task?: unknown }).task;
						const status =
							task.state.status === "terminal"
								? task.state.outcome.status === "completed"
									? "done"
									: task.state.outcome.status === "aborted"
										? "aborted"
										: "failed"
								: task.state.status;
						children.push({
							conversationId: record.id,
							taskId: record.owner.taskId,
							callId,
							task: typeof taskText === "string" ? taskText : "Subagent",
							status,
						});
					}
					cursor = page.next;
				} while (cursor !== undefined);
				return children;
			};
			const child = async (id: number, ctx: Context) => {
				if (!Number.isSafeInteger(id) || id < 1 || !(await list(ctx)).some((row) => row.conversationId === id)) {
					throw new Error("Not a subagent of this session");
				}
				const conversation = await harness.conversation(id as ConversationId, ctx);
				if (conversation === undefined) throw new Error("Subagent conversation no longer exists");
				return conversation;
			};
			env.provide(Subagents, {
				activity,
				configuration,
				list,
				async view(id, ctx) {
					const state = await (await child(id, ctx)).viewState(ctx);
					try {
						return state.value;
					} finally {
						state.dispose();
					}
				},
				async open(id, ctx) {
					// Validation precedes the cache: guessed root or foreign ids must never open a stream.
					const conversation = await child(id, ctx);
					let opening = mounted.get(id);
					if (opening === undefined) {
						if (mounted.size >= 64) throw new Error("Too many child transcripts are open in this session");
						opening = (async () => {
							const state = await conversation.viewState(ctx);
							try {
								transcripts.spawn(String(id), { state });
								env.own(() => state.dispose());
							} catch (error) {
								state.dispose();
								throw error;
							}
						})();
						mounted.set(id, opening);
					}
					try {
						await opening;
					} catch (error) {
						if (mounted.get(id) === opening) mounted.delete(id);
						throw error;
					}
				},
				async page(id, request, ctx) {
					return createHistory(await child(id, ctx)).page(request, ctx);
				},
				async setEnabled(next, ctx) {
					if (typeof next !== "boolean") throw new Error("setEnabled requires a boolean");
					await harness.commit(async (tx) => {
						const state = await tx.doc(AgentDoc, rootId);
						const current = state.extensions;
						if (Array.isArray(current)) {
							state.extensions = next
								? [...new Set([...current, Subagent.name])]
								: current.filter((name) => name !== Subagent.name);
						} else {
							const previous = current as { add?: string[]; remove?: string[] } | undefined;
							state.extensions = {
								...(previous?.add === undefined && !next
									? {}
									: {
											add: next
												? [...new Set([...(previous?.add ?? []), Subagent.name])]
												: previous?.add?.filter((name) => name !== Subagent.name),
										}),
								remove: next
									? (previous?.remove ?? []).filter((name) => name !== Subagent.name)
									: [...new Set([...(previous?.remove ?? []), Subagent.name])],
							};
						}
					}, ctx);
					const effective = await selected(ctx);
					configuration.change(ctx, (draft) => {
						draft.enabled = effective;
					});
				},
			});
		},
	});
}
