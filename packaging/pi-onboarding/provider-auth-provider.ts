import { randomUUID } from "node:crypto";
import { type MutableReplicatedState, replicatedState } from "@earendil-works/chord";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { AuthEvent, AuthInteraction, AuthPrompt, CredentialInfo, LoginOptions } from "@earendil-works/pi-ai";
import type { ProviderAuth, ProviderAuthState, SignInChallenge, SignInProvider } from "./provider-auth.ts";

const INITIAL: ProviderAuthState = {
	credentialsKnown: false,
	credentialLookupFailed: false,
	hasExistingCredentials: false,
	existingProviders: [],
	attempt: null,
	provider: null,
	status: "idle",
	message: "",
	url: null,
	deviceCode: null,
	challenge: null,
};

/** Each client gets its own login UI and pending challenges; credentials stay in Pi's storage. */
export function createProviderAuthService(options: {
	getRuntime(): Promise<{
		login(provider: string, type: "oauth", interaction: AuthInteraction, options: LoginOptions): Promise<unknown>;
		listCredentials(): Promise<readonly CredentialInfo[]>;
		logout(provider: string): Promise<void>;
		refresh(options: {
			providers: string[];
			allowNetwork: false;
			signal: AbortSignal;
		}): Promise<{ errors: ReadonlyMap<string, unknown> }>;
		getProviderAuthStatus(provider: string): { configured: boolean };
		getAvailableSnapshot(): readonly { provider: string }[];
	}>;
	getDeviceId(): string;
}): { readonly service: ProviderAuth; dispose(): void } {
	const state: MutableReplicatedState<ProviderAuthState> = replicatedState(INITIAL);
	let active: { id: string; abort: AbortController; reject?: (error: Error) => void } | undefined;
	let disposed = false;
	let removing = false;
	let credentialScan = 0;
	const refreshCredentials = async (): Promise<void> => {
		const scan = ++credentialScan;
		try {
			const runtime = await options.getRuntime();
			const credentials = await runtime.listCredentials();
			if (!disposed && scan === credentialScan)
				state.change(BACKGROUND_CONTEXT, (draft) =>
					Object.assign(draft, {
						credentialsKnown: true,
						credentialLookupFailed: false,
						hasExistingCredentials: credentials.length > 0,
						existingProviders: credentials
							.filter(
								(entry) =>
									entry.type === "oauth" &&
									(entry.providerId === "anthropic" || entry.providerId === "openai-codex"),
							)
							.map((entry) => entry.providerId as SignInProvider),
					}),
				);
		} catch {
			// Fail closed: unreadable auth storage must not look like an empty profile.
			if (!disposed && scan === credentialScan)
				state.change(BACKGROUND_CONTEXT, (draft) => Object.assign(draft, { credentialLookupFailed: true }));
		}
	};
	void refreshCredentials();
	const isCurrent = (id: string): boolean => !disposed && active?.id === id;
	const publish = (id: string, update: Partial<ProviderAuthState>): void => {
		if (!isCurrent(id)) return;
		state.change(BACKGROUND_CONTEXT, (draft) => Object.assign(draft, update));
	};
	const stop = (): void => {
		const old = active;
		active = undefined;
		old?.abort.abort();
		old?.reject?.(new Error("Sign-in cancelled"));
	};
	const prompt = (id: string, request: AuthPrompt): Promise<string> => {
		if (!isCurrent(id)) return Promise.reject(new Error("Sign-in cancelled"));
		const challenge: SignInChallenge = {
			id: randomUUID(),
			type: request.type,
			message: request.message,
			...("placeholder" in request && request.placeholder !== undefined ? { placeholder: request.placeholder } : {}),
			...(request.type === "select" ? { options: request.options.map((o) => ({ ...o })) } : {}),
		};
		// Prompt text/options are provider-controlled UI copy; never publish a response or key.
		publish(id, { status: "prompt", challenge });
		return new Promise<string>((resolve, reject) => {
			if (!isCurrent(id) || request.signal?.aborted) {
				reject(new Error("Sign-in cancelled"));
				return;
			}
			const signal = request.signal;
			const onAbort = (): void => {
				responses.delete(challenge.id);
				if (active?.reject === rejectPrompt) active.reject = undefined;
				signal?.removeEventListener("abort", onAbort);
				publish(id, { challenge: null, status: "waiting" });
				reject(new Error("Sign-in cancelled"));
			};
			const rejectPrompt = (error: Error): void => {
				responses.delete(challenge.id);
				signal?.removeEventListener("abort", onAbort);
				reject(error);
			};
			signal?.addEventListener("abort", onAbort, { once: true });
			active!.reject = rejectPrompt;
			responses.set(challenge.id, (answer) => {
				signal?.removeEventListener("abort", onAbort);
				if (active?.reject === rejectPrompt) active.reject = undefined;
				resolve(answer);
			});
		});
	};
	const responses = new Map<string, (answer: string) => void>();
	const notify = (id: string, event: AuthEvent): void => {
		if (!isCurrent(id)) return;
		switch (event.type) {
			case "auth_url":
				publish(id, {
					status: "waiting",
					url: event.url,
					message: event.instructions ?? "Continue in your browser.",
				});
				break;
			case "device_code":
				publish(id, {
					status: "waiting",
					url: event.verificationUri,
					deviceCode: event.userCode,
					message: "Enter this code on the provider's sign-in page.",
				});
				break;
			case "info":
			case "progress":
				publish(id, { message: event.message });
				break;
		}
	};
	const service: ProviderAuth = {
		state,
		async start(provider) {
			if (removing) throw new Error("Wait for connection removal to finish");
			if (disposed) throw new Error("Sign-in is not available");
			if (provider !== "anthropic" && provider !== "openai-codex") {
				throw new Error("This provider is not available for sign-in");
			}
			stop();
			responses.clear();
			const id = randomUUID();
			const abort = new AbortController();
			active = { id, abort };
			state.change(BACKGROUND_CONTEXT, (draft) =>
				Object.assign(draft, {
					attempt: id,
					provider: provider as SignInProvider,
					status: "connecting",
					message: "Connecting…",
					url: null,
					deviceCode: null,
					challenge: null,
				}),
			);
			void (async () => {
				try {
					const runtime = await options.getRuntime();
					if (!isCurrent(id)) return;
					await runtime.login(
						provider,
						"oauth",
						{
							signal: abort.signal,
							prompt: (request) => prompt(id, request),
							notify: (event) => notify(id, event),
						},
						{ getDeviceId: options.getDeviceId },
					);
					await refreshCredentials();
					publish(id, {
						status: "done",
						message: "Connected. Choose a model to continue.",
						challenge: null,
						url: null,
						deviceCode: null,
					});
				} catch {
					// Provider errors may contain response URLs or tokens; never replicate them.
					publish(id, {
						status: "failed",
						message: "Could not complete sign-in. Try again.",
						challenge: null,
						url: null,
						deviceCode: null,
					});
				} finally {
					if (active?.id === id) active.reject = undefined;
				}
			})();
			return id;
		},
		async reuse(provider) {
			if (removing) throw new Error("Wait for connection removal to finish");
			if (disposed) throw new Error("Sign-in is not available");
			if (provider !== "anthropic" && provider !== "openai-codex") {
				throw new Error("This provider is not available for sign-in");
			}
			stop();
			responses.clear();
			const id = randomUUID();
			const abort = new AbortController();
			active = { id, abort };
			state.change(BACKGROUND_CONTEXT, (draft) =>
				Object.assign(draft, {
					attempt: id,
					provider,
					status: "connecting",
					message: "Checking your saved connection…",
					url: null,
					deviceCode: null,
					challenge: null,
				}),
			);
			void (async () => {
				try {
					const runtime = await options.getRuntime();
					const saved = await runtime.listCredentials();
					if (!isCurrent(id)) return;
					if (!saved.some((c) => c.providerId === provider && c.type === "oauth"))
						throw new Error("no saved OAuth credential");
					const refresh = await runtime.refresh({
						providers: [provider],
						allowNetwork: false,
						signal: abort.signal,
					});
					if (
						refresh.errors.has(provider) ||
						!runtime.getProviderAuthStatus(provider).configured ||
						!runtime.getAvailableSnapshot().some((model) => model.provider === provider)
					) {
						throw new Error("Saved connection is not ready");
					}
					publish(id, { status: "done", message: "Connected. Choose a project to continue." });
				} catch {
					publish(id, {
						status: "failed",
						message: "The saved connection is not ready. Sign in again to continue.",
					});
				}
			})();
			return id;
		},
		async remove(provider) {
			if (removing) throw new Error("Wait for connection removal to finish");
			if (disposed) throw new Error("Sign-in is not available");
			if (provider !== "anthropic" && provider !== "openai-codex")
				throw new Error("This provider is not available for sign-in");
			removing = true;
			stop();
			responses.clear();
			try {
				const runtime = await options.getRuntime();
				await runtime.logout(provider);
				await refreshCredentials();
				if (!disposed) state.change(BACKGROUND_CONTEXT, (draft) => Object.assign(draft, {
					attempt: null, provider: null, status: "idle",
					message: "Saved connection removed.", url: null, deviceCode: null, challenge: null,
				}));
			} catch {
				throw new Error("Could not remove the saved connection. Try again.");
			} finally {
				removing = false;
			}
		},
		async answer(attempt, challenge, response) {
			if (!isCurrent(attempt) || state.value?.challenge?.id !== challenge || response.length > 8192) {
				throw new Error("The sign-in question has expired");
			}
			if (
				state.value.challenge.type === "select" &&
				!state.value.challenge.options?.some((o) => o.id === response)
			) {
				throw new Error("The selected option is not available");
			}
			const accept = responses.get(challenge);
			if (!accept) throw new Error("The sign-in question has expired");
			responses.delete(challenge);
			publish(attempt, { challenge: null, status: "waiting" });
			accept(response);
		},
		async cancel(attempt) {
			if (!isCurrent(attempt)) return;
			stop();
			responses.clear();
			state.change(BACKGROUND_CONTEXT, (draft) =>
				Object.assign(draft, {
					attempt: null,
					provider: null,
					status: "idle",
					message: "",
					url: null,
					deviceCode: null,
					challenge: null,
				}),
			);
		},
	};
	return {
		service,
		dispose() {
			if (disposed) return;
			stop();
			responses.clear();
			disposed = true;
		},
	};
}
