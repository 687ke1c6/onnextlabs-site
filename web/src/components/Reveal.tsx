import type { JSX } from "solid-js";
import { createSignal, onCleanup, onMount } from "solid-js";
import { cn } from "~/lib/utils";

type RevealProps = JSX.HTMLAttributes<HTMLDivElement> & {
  from?: "left" | "right";
  /** Stagger delay in ms. */
  delay?: number;
};

/** Slides + fades its children in the first time they scroll into view. */
export function Reveal(props: RevealProps) {
  const [shown, setShown] = createSignal(false);
  let el!: HTMLDivElement;

  onMount(() => {
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setShown(true);
          io.disconnect();
        }
      },
      { threshold: 0.2 },
    );
    io.observe(el);
    onCleanup(() => io.disconnect());
  });

  return (
    <div
      ref={el}
      class={cn(
        "transition-all duration-700 ease-out motion-reduce:transition-none",
        shown()
          ? "translate-x-0 opacity-100"
          : props.from === "right"
            ? "translate-x-24 opacity-0"
            : "-translate-x-24 opacity-0",
        props.class,
      )}
      style={{ "transition-delay": `${props.delay ?? 0}ms` }}
    >
      {props.children}
    </div>
  );
}
