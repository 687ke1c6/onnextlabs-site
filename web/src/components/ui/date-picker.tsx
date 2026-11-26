/** `YYYY-MM-DD` moved by `days`. */
export function addDays(iso: string, days: number): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/** Keys that still work: moving focus, and opening the picker. */
const ALLOWED = new Set(["Tab", "Escape"]);
const OPENS = new Set(["Enter", " ", "ArrowDown"]);

/**
 * A date chosen from the browser's date picker only: typing into it is blocked, so a half-typed year (0002)
 * can't slip through, and dates outside `min`–`max` are never reported. Clicking it, or Enter / Space / ↓,
 * opens the picker. With no value, the picker opens on `opensOn` (e.g. the row's own date) instead of today.
 */
export function DatePicker(props: {
  value: string;
  onChange: (value: string) => void;
  /** `YYYY-MM-DD` bounds, inclusive. */
  min?: string;
  max?: string;
  /** `YYYY-MM-DD` the picker shows when there's no value yet (default: today). */
  opensOn?: string;
  id?: string;
  class?: string;
  disabled?: boolean;
  "aria-label"?: string;
}) {
  // The browser's picker opens on the input's value, so with none yet it's given `opensOn` while
  // the picker is open. Nothing picked (it's left without a change): it goes back to empty.
  let picked = false;
  const open = (el: HTMLInputElement) => {
    picked = false;
    if (!props.value && props.opensOn) el.value = props.opensOn;
    try {
      el.showPicker();
    } catch {
      // not supported, or not from a user gesture: the browser's own button still opens it
    }
  };
  return (
    <input
      type="date"
      id={props.id}
      class={props.class}
      aria-label={props["aria-label"]}
      value={props.value}
      min={props.min}
      max={props.max}
      disabled={props.disabled}
      onClick={(e) => open(e.currentTarget)}
      onKeyDown={(e) => {
        if (ALLOWED.has(e.key)) return;
        e.preventDefault();
        if (OPENS.has(e.key)) open(e.currentTarget);
      }}
      onBlur={(e) => {
        if (!picked && e.currentTarget.value !== props.value) e.currentTarget.value = props.value;
      }}
      onPaste={(e) => e.preventDefault()}
      onDrop={(e) => e.preventDefault()}
      onChange={(e) => {
        const v = e.currentTarget.value;
        if (!v || (props.min && v < props.min) || (props.max && v > props.max)) {
          e.currentTarget.value = props.value; // put it back
          return;
        }
        picked = true;
        props.onChange(v);
      }}
    />
  );
}
