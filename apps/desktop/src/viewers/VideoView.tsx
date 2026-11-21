import { Show } from "solid-js";
import { api } from "../api";
import { extOfName } from "./kinds";

/** Video/audio phát trực tiếp từ store (Range đã hỗ trợ, tua được). */
export default function VideoView(props: { path: string }) {
  const url = () => api.streamUrl(props.path);
  const audio = () => ["mp3", "m4a", "ogg", "oga", "opus", "wav", "flac"].includes(extOfName(props.path));
  return (
    <div class="media-wrap">
      <Show when={audio()} fallback={<video ref={(el) => el?.setAttribute("playsinline", "")} src={url()} controls preload="metadata" />}>
        <audio src={url()} controls preload="metadata" />
      </Show>
      <p class="muted sm">{props.path.split("/").pop()}</p>
    </div>
  );
}
