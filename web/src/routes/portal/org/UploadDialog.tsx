import { createEffect, createResource, createSignal, For, on, Show } from "solid-js";
import FolderPlus from "lucide-solid/icons/folder-plus";
import Loader2 from "lucide-solid/icons/loader-circle";
import UploadCloud from "lucide-solid/icons/cloud-upload";
import X from "lucide-solid/icons/x";
import { FileTypeIcon } from "~/components/FileTypeIcon";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import {
  type DocumentItem,
  fetchAllFolders,
  folderNameError,
  formatBytes,
  uploadDocument,
} from "~/lib/documents";
import { cn, settled } from "~/lib/utils";

const folderLabel = (path: string) => (path ? `Documents / ${path.split("/").join(" / ")}` : "Documents (top level)");

export function UploadDialog(props: {
  org: string;
  open: boolean;
  initialFolder: string;
  maxBytes: number;
  onClose: () => void;
  onUploaded: (doc: DocumentItem) => void;
}) {
  let dialog!: HTMLDialogElement;
  let fileInput!: HTMLInputElement;

  const [file, setFile] = createSignal<File | null>(null);
  const [parent, setParent] = createSignal("");
  const [newFolder, setNewFolder] = createSignal<string | null>(null); // null = not creating one
  const [dragging, setDragging] = createSignal(false);
  const [progress, setProgress] = createSignal<number | null>(null); // null = not uploading
  const [error, setError] = createSignal<string | null>(null);

  const [folders] = createResource(() => (props.open ? props.org : undefined), fetchAllFolders);

  // Reset each time the dialog opens, starting in the folder being browsed.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          setFile(null);
          setParent(props.initialFolder);
          setNewFolder(null);
          setProgress(null);
          setError(null);
          dialog.showModal();
        } else if (dialog.open) {
          dialog.close();
        }
      },
    ),
  );

  const destination = () => {
    const nf = newFolder();
    if (nf === null) return parent();
    return parent() ? `${parent()}/${nf.trim()}` : nf.trim();
  };
  const newFolderError = () => (newFolder() === null ? null : folderNameError(newFolder()!));
  const fileError = () => {
    const f = file();
    if (!f) return null;
    if (f.size === 0) return "That file is empty.";
    if (f.size > props.maxBytes) return `That file is ${formatBytes(f.size)}; the limit is ${formatBytes(props.maxBytes)}.`;
    return null;
  };
  const uploading = () => progress() !== null;
  const canUpload = () => !!file() && !fileError() && !newFolderError() && !uploading();

  const pick = (files: FileList | null | undefined) => {
    setError(null);
    if (!files?.length) return;
    if (files.length > 1) setError("One file at a time for now; using the first.");
    setFile(files[0]);
  };

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    const f = file();
    if (!f || !canUpload()) return;
    setError(null);
    setProgress(0);
    try {
      const doc = await uploadDocument(props.org, f, destination(), setProgress);
      props.onUploaded(doc);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Upload failed.");
    } finally {
      setProgress(null);
    }
  };

  const close = () => {
    if (!uploading()) props.onClose();
  };

  return (
    <dialog
      ref={dialog}
      onClose={() => props.open && props.onClose()}
      onCancel={(e) => uploading() && e.preventDefault()}
      class="m-auto w-[calc(100%-2rem)] max-w-lg rounded-2xl border border-border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/60 backdrop:backdrop-blur-sm"
      aria-labelledby="upload-title"
    >
      <form onSubmit={submit} class="flex flex-col gap-5 p-6">
        <div class="flex items-start justify-between gap-4">
          <div>
            <h2 id="upload-title" class="text-lg font-semibold tracking-tight">Upload document</h2>
            <p class="mt-1 text-sm text-muted-foreground">Up to {formatBytes(props.maxBytes)}. Anyone with the link can download it.</p>
          </div>
          <button
            type="button"
            onClick={close}
            disabled={uploading()}
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
            aria-label="Close"
          >
            <X size={18} />
          </button>
        </div>

        {/* Drop zone (also a button that opens the file picker) */}
        <button
          type="button"
          onClick={() => fileInput.click()}
          onDragEnter={(e) => {
            e.preventDefault();
            setDragging(true);
          }}
          onDragOver={(e) => {
            e.preventDefault();
            if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
          }}
          onDragLeave={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
          }}
          onDrop={(e) => {
            e.preventDefault();
            setDragging(false);
            pick(e.dataTransfer?.files);
          }}
          disabled={uploading()}
          class={cn(
            "flex flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed px-4 py-8 text-center transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            dragging() ? "border-ring bg-accent/60" : "border-border hover:bg-accent/40",
          )}
        >
          <Show
            when={file()}
            fallback={
              <>
                <UploadCloud size={28} class="text-muted-foreground" />
                <span class="text-sm font-medium">Drop a file here, or click to choose</span>
              </>
            }
          >
            {(f) => (
              <>
                <FileTypeIcon filename={f().name} contentType={f().type} size={28} />
                <span class="max-w-full truncate text-sm font-medium">{f().name}</span>
                <span class="text-xs text-muted-foreground">{formatBytes(f().size)} · click to change</span>
              </>
            )}
          </Show>
        </button>
        <input ref={fileInput} type="file" class="hidden" onChange={(e) => pick(e.currentTarget.files)} />
        <Show when={fileError()}>{(msg) => <p class="-mt-3 text-sm text-red-500">{msg()}</p>}</Show>

        {/* Destination */}
        <div class="flex flex-col gap-2">
          <label for="upload-folder" class="text-sm font-medium">Folder</label>
          <div class="flex gap-2">
            <select
              id="upload-folder"
              // Re-applied when the folder list arrives, so the preselected option sticks.
              value={(settled(folders), parent())}
              onChange={(e) => setParent(e.currentTarget.value)}
              disabled={uploading() || folders.loading}
              class="h-10 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <option value="">{folderLabel("")}</option>
              <For each={settled(folders) ?? (props.initialFolder ? [props.initialFolder] : [])}>
                {(path) => <option value={path}>{folderLabel(path)}</option>}
              </For>
            </select>
            <Show when={newFolder() === null}>
              <Button type="button" variant="outline" onClick={() => setNewFolder("")} disabled={uploading()}>
                <FolderPlus size={16} /> New
              </Button>
            </Show>
          </div>
          <Show when={folders.error}>
            <p class="text-sm text-red-500">Couldn't load folders; you can still upload here or create a new one.</p>
          </Show>

          <Show when={newFolder() !== null}>
            <div class="flex gap-2">
              <Input
                aria-label="New folder name"
                placeholder="New folder name"
                value={newFolder()!}
                onInput={(e) => setNewFolder(e.currentTarget.value)}
                disabled={uploading()}
                ref={(el) => queueMicrotask(() => el.focus())}
                class="flex-1"
              />
              <Button type="button" variant="ghost" onClick={() => setNewFolder(null)} disabled={uploading()}>
                Cancel
              </Button>
            </div>
            <Show when={newFolder()!.trim() && newFolderError()}>{(msg) => <p class="text-sm text-red-500">{msg()}</p>}</Show>
          </Show>
          <p class="text-xs text-muted-foreground">
            Uploading to <span class="font-medium text-foreground">{folderLabel(destination())}</span>
          </p>
        </div>

        <Show when={error()}>{(msg) => <p class="text-sm text-red-500">{msg()}</p>}</Show>

        <Show when={uploading()}>
          <div class="h-1.5 overflow-hidden rounded-full bg-secondary" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((progress() ?? 0) * 100)}>
            <div class="h-full bg-primary transition-[width]" style={{ width: `${(progress() ?? 0) * 100}%` }} />
          </div>
        </Show>

        <div class="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={close} disabled={uploading()}>Cancel</Button>
          <Button type="submit" disabled={!canUpload()}>
            <Show when={uploading()} fallback={<UploadCloud size={16} />}>
              <Loader2 size={16} class="animate-spin" />
            </Show>
            {uploading() ? "Uploading…" : "Upload"}
          </Button>
        </div>
      </form>
    </dialog>
  );
}
