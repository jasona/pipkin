import assert from "node:assert/strict";
import { test } from "node:test";
import { createProviderAuthService } from "./provider-auth-provider.ts";

// Synthetic runtime only: never reads or changes the user's credential store.
test("removal is allowlisted, serialized, and publishes fresh credential metadata", async () => {
	let saved = true;
	let finish: (() => void) | undefined;
	const removed: string[] = [];
	const auth = createProviderAuthService({
		getDeviceId: () => "test-device",
		getRuntime: async () => ({
			login: async () => {},
			listCredentials: async () => saved ? [{ providerId: "anthropic", type: "oauth" } as never] : [],
			logout: async (provider: string) => {
				removed.push(provider);
				await new Promise<void>((resolve) => { finish = resolve; });
				saved = false;
			},
			refresh: async () => ({ errors: new Map() }),
			getProviderAuthStatus: () => ({ configured: saved }),
			getAvailableSnapshot: () => [],
		}),
	});
	try {
		await new Promise((resolve) => setImmediate(resolve));
		assert.deepEqual(auth.service.state.value.existingProviders, ["anthropic"]);
		await assert.rejects(auth.service.remove("other" as never, {} as never), /not available/);
		assert.deepEqual(removed, []);
		const pending = auth.service.remove("anthropic", {} as never);
		await new Promise((resolve) => setImmediate(resolve));
		await assert.rejects(auth.service.start("anthropic", {} as never), /Wait/);
		await assert.rejects(auth.service.remove("anthropic", {} as never), /Wait/);
		finish!();
		await pending;
		assert.deepEqual(removed, ["anthropic"]);
		assert.deepEqual(auth.service.state.value.existingProviders, []);
		assert.equal(auth.service.state.value.hasExistingCredentials, false);
		assert.equal(auth.service.state.value.status, "idle");
	} finally {
		auth.dispose();
	}
});

test("removal errors do not replicate runtime secrets or claim success", async () => {
	const auth = createProviderAuthService({
		getDeviceId: () => "test-device",
		getRuntime: async () => ({
			login: async () => {},
			listCredentials: async () => [{ providerId: "anthropic", type: "oauth" } as never],
			logout: async () => { throw new Error("synthetic-private-token"); },
			refresh: async () => ({ errors: new Map() }),
			getProviderAuthStatus: () => ({ configured: true }),
			getAvailableSnapshot: () => [],
		}),
	});
	try {
		await assert.rejects(auth.service.remove("anthropic", {} as never), /^Error: Could not remove/);
		assert.deepEqual(auth.service.state.value.existingProviders, ["anthropic"]);
		assert.ok(!JSON.stringify(auth.service.state.value).includes("synthetic-private-token"));
	} finally {
		auth.dispose();
	}
});
