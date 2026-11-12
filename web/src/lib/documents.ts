export type DocumentItem = {
  id: number;
  folder: string;
  filename: string;
  size: number;
  content_type: string | null;
  uploaded_at: string;
  uploaded_by: string | null;
  public: boolean;
};

export type FolderListing = {
  folder: string;
  folders: string[];
  documents: DocumentItem[];
  max_upload_bytes: number;
};

async function errorMessage(res: Response, fallback: string): Promise<string> {
  try {
    const body = await res.json();
    if (typeof body?.error === "string") return body.error;
  } catch {
    // not JSON
  }
  return fallback;
}

const base = (org: string) => `/api/orgs/${encodeURIComponent(org)}/documents`;

export async function fetchFolder(org: string, folder: string): Promise<FolderListing> {
  const res = await fetch(`${base(org)}?folder=${encodeURIComponent(folder)}`);
  if (!res.ok) throw new Error(await errorMessage(res, `Couldn't load this folder (${res.status}).`));
  return res.json();
}

/** Every folder path, e.g. ["Brand", "Brand/Logos"]. */
export async function fetchAllFolders(org: string): Promise<string[]> {
  const res = await fetch(`${base(org)}/folders`);
  if (!res.ok) throw new Error(await errorMessage(res, "Couldn't load folders."));
  return res.json();
}

/** Uploads one file into the org's `folder` (created if new), reporting progress from 0 to 1. */
export function uploadDocument(
  org: string,
  file: File,
  folder: string,
  onProgress: (fraction: number) => void,
): Promise<DocumentItem> {
  const form = new FormData();
  form.append("folder", folder); // must come before the file
  form.append("file", file);

  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", base(org));
    xhr.responseType = "json";
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) onProgress(e.loaded / e.total);
    };
    xhr.onload = () => {
      if (xhr.status === 201) return resolve(xhr.response as DocumentItem);
      const msg = typeof xhr.response?.error === "string" ? xhr.response.error : `Upload failed (${xhr.status}).`;
      reject(new Error(msg));
    };
    xhr.onerror = () => reject(new Error("Upload failed: network error."));
    xhr.send(form);
  });
}

export const downloadUrl = (doc: DocumentItem) => `/api/documents/${doc.id}/download`;

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** Mirrors the server's name rules so mistakes show up before uploading. */
export function folderNameError(name: string): string | null {
  const n = name.trim();
  if (!n) return "Enter a folder name.";
  if (n.startsWith(".")) return "Folder names can't start with a dot.";
  if (/[/\\\p{Cc}]/u.test(n)) return "Folder names can't contain slashes.";
  if (new TextEncoder().encode(n).length > 200) return "That name is too long.";
  return null;
}
