import "solid-js";

type AltchaState = "unverified" | "verifying" | "verified" | "expired" | "error" | "code";

declare global {
  interface AltchaWidgetElement extends HTMLElement {
    verify(): Promise<unknown>;
    reset(): void;
    getState(): AltchaState;
  }
  type AltchaStateEvent = CustomEvent<{ state: AltchaState; payload?: string }>;
}

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      "altcha-widget": HTMLAttributes<AltchaWidgetElement> & {
        "attr:challenge"?: string;
        "attr:auto"?: "off" | "onfocus" | "onload" | "onsubmit";
        "attr:display"?: "standard" | "bar" | "floating" | "overlay" | "invisible";
      };
    }
    interface CustomEvents {
      statechange: AltchaStateEvent;
    }
  }
}
