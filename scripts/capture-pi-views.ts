// Capture REAL ConversationView values from Pi's durable harness, driven by scripted faux
// responses, as JSON fixtures for Pipkin's mapper tests. Imports Pi's source by absolute path and
// modifies nothing in the Pi checkout. Needs the Pi checkout's dependencies (npm ci there).
//
//   PI_REPO=~/coding/pi node --import file://$PI_REPO/packages/coding-agent/src/experimental/source-resolver.ts \
//     scripts/capture-pi-views.ts fixtures/pi
//
// The captured values are what a Pi server sends as the `pi.transcript` state.
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const repo = resolve(process.env.PI_REPO ?? `${process.env.HOME}/coding/pi`);
const out = resolve(process.argv[2] ?? "fixtures/pi");
mkdirSync(out, { recursive: true });
const PI = `${repo}/packages`;
const ai = await import(`${PI}/ai/src/index.ts`);
const ctx = await import(`${PI}/chord/src/context/index.ts`);
const support = await import(`${PI}/coding-agent/test/experimental-durable-support.ts`);
const provider = await import(`${PI}/coding-agent/src/experimental/services/agent-controller-provider.ts`);

const { fauxAssistantMessage, fauxText, fauxThinking, fauxToolCall } = ai;
const BG = ctx.BACKGROUND_CONTEXT;

async function snapshot(name: string, conversation: any) {
	const view = await conversation.viewState(BG);
	try {
		writeFileSync(join(out, `${name}.json`), `${JSON.stringify(view.value, null, 2)}\n`);
		console.log(name, "entries:", view.value.entries.length, "docs:", Object.keys(view.value.docs).join(","));
	} finally {
		view.dispose();
	}
}

async function run(name: string, responses: unknown[], prompts: string[]) {
	const { harness, conversation, close } = await support.openFauxConversation(responses);
	try {
		const controller = provider.createAgentController(harness, conversation);
		for (const message of prompts) {
			const r = await controller.prompt({ message, images: null }, BG);
			if (r.accepted) await controller.waitForPrompt(r.operationId, BG);
		}
		await snapshot(name, conversation);
	} finally {
		await close();
	}
}

await run("text", [fauxAssistantMessage("hello back")], ["hello"]);
await run(
	"thinking",
	[fauxAssistantMessage([fauxThinking("weigh the options"), fauxText("Here is my answer.")])],
	["think about it"],
);
await run(
	"tool-call",
	[
		fauxAssistantMessage([fauxText("Reading it."), fauxToolCall("read_file", { path: "a.rs" }, { id: "call-1" })], {
			stopReason: "toolUse",
		}),
		fauxAssistantMessage("Done reading."),
	],
	["read a.rs"],
);
await run("error", [fauxAssistantMessage([], { stopReason: "error", errorMessage: "provider unavailable" })], ["go"]);
await run("multi-turn", [fauxAssistantMessage("first reply"), fauxAssistantMessage("second reply")], ["one", "two"]);

// A run in progress: the provider has been asked and has not answered.
{
	const pending = support.pendingResponse();
	const { harness, conversation, close } = await support.openFauxConversation([pending.step]);
	try {
		const controller = provider.createAgentController(harness, conversation);
		await controller.prompt({ message: "work on it", images: null }, BG);
		await pending.reached;
		await snapshot("busy", conversation);
		await controller.abort(BG);
	} finally {
		await close();
	}
}

// REAL WIRE STREAMS. Reproduce the server's path exactly: the Transcript provider behind Chord's
// remote endpoint, with one fresh state encoder per subscription that encodes the snapshot and
// then every update (packages/server/src/server.ts). Each fixture is
// { snapshot, updates, final }: the wire subscribe result, the wire updates in order, and the
// state the replica must end at.
const chord = await import(`${PI}/chord/src/index.ts`);
const transcriptService = await import(`${PI}/coding-agent/src/experimental/services/transcript.ts`);

async function stream(name: string, responses: unknown[], prompts: string[], abortAfterFirst = false) {
	const pending = abortAfterFirst ? support.pendingResponse() : undefined;
	const { harness, conversation, close } = await support.openFauxConversation(
		pending ? [pending.step] : responses,
	);
	const state = await conversation.viewState(BG);
	try {
		const remote = new chord.RemoteServiceProvider([{ service: transcriptService.Transcript, mode: "singleton" }]);
		remote.provide(transcriptService.Transcript, { state });
		const endpoint = chord.createRemoteServiceEndpoint(remote);
		const raw: unknown[] = [];
		const snapshot = await endpoint.invoke(
			chord.createServiceSubscribeCall("sub-1", transcriptService.Transcript.id, "singleton"),
			(_id: string, update: unknown) => {
				raw.push(update);
			},
			BG,
		);
		const encoder = chord.createServiceStateEncoder();
		const wireSnapshot = encoder.encodeSnapshot(chord.parseServiceSubscriptionSnapshot(snapshot));
		const controller = provider.createAgentController(harness, conversation);
		for (const message of prompts) {
			const r = await controller.prompt({ message, images: null }, BG);
			if (pending) {
				await pending.reached;
				await controller.abort(BG);
			} else if (r.accepted) await controller.waitForPrompt(r.operationId, BG);
		}
		await new Promise((resolve) => setTimeout(resolve, 100));
		const updates = raw.map((update) => encoder.encodeUpdate(update as never));
		writeFileSync(
			join(out, `stream-${name}.json`),
			`${JSON.stringify({ snapshot: wireSnapshot, updates, final: state.value }, null, 2)}\n`,
		);
		console.log(`stream-${name}`, "updates:", updates.length);
		endpoint.dispose();
	} finally {
		state.dispose();
		await close();
	}
}

const longText = Array.from({ length: 80 }, (_, i) => `word${i}`).join(" ");
await stream("text", [fauxAssistantMessage("hello back")], ["hello"]);
await stream("long", [fauxAssistantMessage(longText)], ["write something long"]);
await stream(
	"tool",
	[
		fauxAssistantMessage([fauxText("Reading it."), fauxToolCall("read_file", { path: "a.rs" }, { id: "call-1" })], {
			stopReason: "toolUse",
		}),
		fauxAssistantMessage("Done reading."),
	],
	["read a.rs"],
);
await stream("multi", [fauxAssistantMessage("first reply"), fauxAssistantMessage("second reply")], ["one", "two"]);
await stream("abort", [], ["start then stop"], true);
console.log("done");
process.exit(0);
