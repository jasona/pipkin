import { type Context, defineService, type ReplicatedState } from "@earendil-works/chord";

/** OAuth-only setup, scoped to one connected local client. No credentials are replicated. */
export type SignInProvider = "anthropic" | "openai-codex";

export interface SignInChallenge {
	id: string;
	type: "text" | "secret" | "select" | "manual_code";
	message: string;
	placeholder?: string;
	options?: readonly { id: string; label: string; description?: string }[];
}

export interface ProviderAuthState {
	/** Non-secret metadata used to keep an existing user's workspace. */
	credentialsKnown: boolean;
	credentialLookupFailed: boolean;
	hasExistingCredentials: boolean;
	/** Only subscription OAuth accounts may be reused within the wizard. */
	existingProviders: readonly SignInProvider[];
	attempt: string | null;
	provider: SignInProvider | null;
	status: "idle" | "connecting" | "waiting" | "prompt" | "done" | "failed";
	message: string;
	url: string | null;
	deviceCode: string | null;
	challenge: SignInChallenge | null;
}

export interface ProviderAuth {
	readonly state: ReplicatedState<ProviderAuthState>;
	/** Starts the provider's OAuth flow; returns before any browser/device interaction. */
	start(provider: SignInProvider, context: Context): Promise<string>;
	/** Explicitly reuse a saved subscription OAuth credential after Pi validates availability. */
	reuse(provider: SignInProvider, context: Context): Promise<string>;
	/** The response is handled in memory only. Never put it in a replicated document. */
	answer(attempt: string, challenge: string, response: string, context: Context): Promise<void>;
	cancel(attempt: string, context: Context): Promise<void>;
	/** Remove the saved provider credential locally; does not cancel a subscription. */
	remove(provider: SignInProvider, context: Context): Promise<void>;
}

export const ProviderAuth = defineService<ProviderAuth>("pi.provider-auth");
