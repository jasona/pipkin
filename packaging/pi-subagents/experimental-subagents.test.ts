import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServiceSubscribeCall } from "@earendil-works/chord";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { createModels, fauxAssistantMessage, fauxProvider, fauxText, fauxToolCall } from "@earendil-works/pi-ai";
import { createRegistry, Harness } from "@earendil-works/pi-durable";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";
import { afterEach, describe, expect, test, vi } from "vitest";
import { Subagent } from "../src/experimental/durable/subagent.ts";
import { createSessionWorkerServices } from "../src/experimental/services/worker.ts";
import { pendingResponse } from "./experimental-durable-support.ts";

const scope = { serverConnectionId: "server", attachmentId: "attachment" };
const directories = new Set<string>();
afterEach(async () => {
	for (const directory of directories) await rm(directory, { recursive: true, force: true });
	directories.clear();
});

describe("experimental subagent service", () => {
	test("toggles the root tool durably without touching a different session", async () => {
		const directory = await mkdtemp(join(tmpdir(), "pi-subagents-service-"));
		directories.add(directory);
		const path = join(directory, "session.sqlite");
		const registry = createRegistry();
		registry.install(Subagent);
		const models = createModels();
		let harness = await Harness.open(await openNodeSqliteStorage(path), { models, registry }, BACKGROUND_CONTEXT);
		let root = await harness.root(BACKGROUND_CONTEXT);
		let services = await createSessionWorkerServices({
			harness,
			conversation: root,
			modelRuntime: undefined,
			publish: vi.fn(async () => {}),
		});
		try {
			const list = await services.invoke(
				{ serviceId: "pi.subagents", member: "list", args: [] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(list).toEqual([]);
			expect((await root.agent(BACKGROUND_CONTEXT)).tools.map((tool) => tool.name)).toContain("subagent");
			await services.invoke(
				{ serviceId: "pi.subagents", member: "setEnabled", args: [false] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect((await root.agent(BACKGROUND_CONTEXT)).tools.map((tool) => tool.name)).not.toContain("subagent");
			await expect(
				services.invoke({ serviceId: "pi.subagents", member: "view", args: [1] }, scope, BACKGROUND_CONTEXT),
			).rejects.toThrow();
		} finally {
			await services.dispose();
			await harness.close(BACKGROUND_CONTEXT);
		}
		harness = await Harness.open(await openNodeSqliteStorage(path), { models, registry }, BACKGROUND_CONTEXT);
		root = await harness.root(BACKGROUND_CONTEXT);
		services = await createSessionWorkerServices({
			harness,
			conversation: root,
			modelRuntime: undefined,
			publish: vi.fn(async () => {}),
		});
		try {
			expect((await root.agent(BACKGROUND_CONTEXT)).tools.map((tool) => tool.name)).not.toContain("subagent");
			await services.invoke(
				{ serviceId: "pi.subagents", member: "setEnabled", args: [true] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect((await root.agent(BACKGROUND_CONTEXT)).tools.map((tool) => tool.name)).toContain("subagent");
		} finally {
			await services.dispose();
			await harness.close(BACKGROUND_CONTEXT);
		}
	});

	test("shows an active child and keeps its history after the parent is aborted", async () => {
		const directory = await mkdtemp(join(tmpdir(), "pi-subagents-service-"));
		directories.add(directory);
		const registry = createRegistry();
		registry.install(Subagent);
		const faux = fauxProvider();
		const pending = pendingResponse();
		faux.setResponses([
			fauxAssistantMessage([fauxToolCall("subagent", { task: "Long task" }, { id: "call-2" })], {
				stopReason: "toolUse",
			}),
			pending.step,
			fauxAssistantMessage([fauxText("Stopped")]),
		]);
		const models = createModels();
		models.setProvider(faux.provider);
		const harness = await Harness.open(
			await openNodeSqliteStorage(join(directory, "session.sqlite")),
			{ models, registry },
			BACKGROUND_CONTEXT,
		);
		const model = faux.getModel();
		const root = await harness.root(BACKGROUND_CONTEXT, {
			agent: { model: { provider: model.provider, modelId: model.id } },
		});
		const publish = vi.fn(async () => {});
		const services = await createSessionWorkerServices({
			harness,
			conversation: root,
			modelRuntime: undefined,
			publish,
		});
		try {
			await root.submit({ type: "input", content: "Delegate" }, BACKGROUND_CONTEXT);
			await pending.reached;
			const rows = await services.invoke(
				{ serviceId: "pi.subagents", member: "list", args: [] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(rows).toMatchObject([{ task: "Long task", status: "running" }]);
			if (!Array.isArray(rows) || rows[0] === null || typeof rows[0] !== "object" || Array.isArray(rows[0])) {
				throw new Error("Missing child");
			}
			const child = rows[0].conversationId;
			if (typeof child !== "number") throw new Error("Missing child id");
			await services.invoke({ serviceId: "pi.subagents", member: "open", args: [child] }, scope, BACKGROUND_CONTEXT);
			await services.invoke(
				createServiceSubscribeCall("live-child", "pi.subagent-transcript", "keyed"),
				scope,
				BACKGROUND_CONTEXT,
			);
			const previous = publish.mock.calls.length;
			await root.abort(BACKGROUND_CONTEXT);
			expect(publish.mock.calls.length).toBeGreaterThan(previous);
			const after = await services.invoke(
				{ serviceId: "pi.subagents", member: "list", args: [] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(after).toMatchObject([{ task: "Long task", status: "aborted" }]);
		} finally {
			await services.dispose();
			await harness.close(BACKGROUND_CONTEXT);
		}
	});

	test("lists a completed child and serves only its transcript and history", async () => {
		const directory = await mkdtemp(join(tmpdir(), "pi-subagents-service-"));
		directories.add(directory);
		const registry = createRegistry();
		registry.install(Subagent);
		const faux = fauxProvider();
		faux.setResponses([
			fauxAssistantMessage([fauxToolCall("subagent", { task: "Inspect one file" }, { id: "call-1" })], {
				stopReason: "toolUse",
			}),
			fauxAssistantMessage([fauxText("Child completed")]),
			fauxAssistantMessage([fauxText("Parent completed")]),
		]);
		const models = createModels();
		models.setProvider(faux.provider);
		const harness = await Harness.open(
			await openNodeSqliteStorage(join(directory, "session.sqlite")),
			{ models, registry },
			BACKGROUND_CONTEXT,
		);
		const model = faux.getModel();
		const root = await harness.root(BACKGROUND_CONTEXT, {
			agent: { model: { provider: model.provider, modelId: model.id } },
		});
		const services = await createSessionWorkerServices({
			harness,
			conversation: root,
			modelRuntime: undefined,
			publish: vi.fn(async () => {}),
		});
		try {
			const submission = await root.submit({ type: "input", content: "Delegate this" }, BACKGROUND_CONTEXT);
			expect((await submission.wait(BACKGROUND_CONTEXT)).status).toBe("done");
			const rows = await services.invoke(
				{ serviceId: "pi.subagents", member: "list", args: [] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(rows).toMatchObject([{ callId: "call-1", task: "Inspect one file", status: "done" }]);
			if (!Array.isArray(rows) || rows[0] === null || typeof rows[0] !== "object" || Array.isArray(rows[0])) {
				throw new Error("Missing child id");
			}
			const id = rows[0].conversationId;
			if (typeof id !== "number") throw new Error("Invalid child id");
			await services.invoke({ serviceId: "pi.subagents", member: "open", args: [id] }, scope, BACKGROUND_CONTEXT);
			const stream = await services.invoke(
				createServiceSubscribeCall("child-view", "pi.subagent-transcript", "keyed"),
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(JSON.stringify(stream)).toContain("Child completed");
			const view = await services.invoke(
				{ serviceId: "pi.subagents", member: "view", args: [id] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(JSON.stringify(view)).toContain("Child completed");
			expect(JSON.stringify(view)).not.toContain("Parent completed");
			const history = await services.invoke(
				{ serviceId: "pi.subagents", member: "page", args: [id, { before: null, limit: 20 }] },
				scope,
				BACKGROUND_CONTEXT,
			);
			expect(JSON.stringify(history)).toContain("Inspect one file");
			await expect(
				services.invoke({ serviceId: "pi.subagents", member: "view", args: [root.id] }, scope, BACKGROUND_CONTEXT),
			).rejects.toThrow();
			await expect(
				services.invoke({ serviceId: "pi.subagents", member: "open", args: [root.id] }, scope, BACKGROUND_CONTEXT),
			).rejects.toThrow();
		} finally {
			await services.dispose();
			await harness.close(BACKGROUND_CONTEXT);
		}
	});
});
