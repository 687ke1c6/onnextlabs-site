import { Dynamic } from "solid-js/web";
import type { LucideIcon } from "lucide-solid";
import File from "lucide-solid/icons/file";
import FileArchive from "lucide-solid/icons/file-archive";
import FileBraces from "lucide-solid/icons/file-braces";
import FileCode from "lucide-solid/icons/file-code";
import FileImage from "lucide-solid/icons/file-image";
import FileMusic from "lucide-solid/icons/file-music";
import FilePen from "lucide-solid/icons/file-pen";
import FileSpreadsheet from "lucide-solid/icons/file-spreadsheet";
import FileText from "lucide-solid/icons/file-text";
import FileType from "lucide-solid/icons/file-type";
import FileVideoCamera from "lucide-solid/icons/file-video-camera";
import Presentation from "lucide-solid/icons/presentation";
import { cn } from "~/lib/utils";

type Kind = { label: string; icon: LucideIcon; color: string };

// Colours are mid-tone (500) so they read on both the light and dark portal themes.
const KINDS = {
  pdf: { label: "PDF document", icon: FileText, color: "text-red-500" },
  word: { label: "Word document", icon: FilePen, color: "text-blue-500" },
  sheet: { label: "Spreadsheet", icon: FileSpreadsheet, color: "text-emerald-500" },
  slides: { label: "Presentation", icon: Presentation, color: "text-orange-500" },
  text: { label: "Text file", icon: FileText, color: "text-muted-foreground" },
  image: { label: "Image", icon: FileImage, color: "text-violet-500" },
  video: { label: "Video", icon: FileVideoCamera, color: "text-pink-500" },
  audio: { label: "Audio", icon: FileMusic, color: "text-amber-500" },
  archive: { label: "Archive", icon: FileArchive, color: "text-yellow-600" },
  data: { label: "Data file", icon: FileBraces, color: "text-sky-500" },
  code: { label: "Code", icon: FileCode, color: "text-sky-500" },
  font: { label: "Font", icon: FileType, color: "text-teal-500" },
  other: { label: "File", icon: File, color: "text-muted-foreground" },
} satisfies Record<string, Kind>;

type KindName = keyof typeof KINDS;

const BY_EXTENSION: Record<string, KindName> = {};
const register = (kind: KindName, exts: string) => exts.split(" ").forEach((e) => (BY_EXTENSION[e] = kind));
register("pdf", "pdf");
register("word", "doc docx docm dot dotx odt rtf pages");
register("sheet", "xls xlsx xlsm xlsb xlt xltx ods numbers csv tsv");
register("slides", "ppt pptx pptm pps ppsx pot potx odp key");
register("text", "txt md markdown log");
register("image", "png jpg jpeg gif webp avif svg bmp tif tiff ico heic heif psd ai eps sketch fig");
register("video", "mp4 m4v mov webm avi mkv wmv");
register("audio", "mp3 wav ogg oga m4a flac aac opus");
register("archive", "zip rar 7z tar gz tgz bz2 xz zst");
register("data", "json xml yaml yml toml");
register("code", "html htm css js mjs ts tsx jsx py rs go java sh sql");
register("font", "ttf otf woff woff2");

function kindOf(filename: string, contentType?: string | null): Kind {
  const dot = filename.lastIndexOf(".");
  const ext = dot > 0 ? filename.slice(dot + 1).toLowerCase() : "";
  const byExt = BY_EXTENSION[ext];
  if (byExt) return KINDS[byExt];
  // Unknown extension: fall back to the broad MIME family the browser reported.
  const family = contentType?.split("/")[0];
  if (family === "image" || family === "video" || family === "audio") return KINDS[family];
  if (family === "text") return KINDS.text;
  return KINDS.other;
}

/** Coloured icon for a file, chosen from its extension (or MIME type as a fallback). */
export function FileTypeIcon(props: { filename: string; contentType?: string | null; size?: number; class?: string }) {
  const kind = () => kindOf(props.filename, props.contentType);
  return (
    <span class="inline-flex shrink-0" title={kind().label}>
      <Dynamic
        component={kind().icon}
        size={props.size ?? 18}
        class={cn("shrink-0", kind().color, props.class)}
        aria-hidden="true"
      />
      <span class="sr-only">{kind().label}: </span>
    </span>
  );
}
