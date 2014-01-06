import indexZh from "../documentation/zh-hans/index.md?raw";
import guideEmbed from "../documentation/zh-hans/guide/embed-and-serve.md?raw";
import guideSecurity from "../documentation/zh-hans/guide/serve-security.md?raw";
import guideWire from "../documentation/zh-hans/guide/wire-protocol.md?raw";

export interface DocPage {
    id: string;
    title: string;
    body: string;
}

export const docs: DocPage[] = [
    { id: "index", title: "概览", body: indexZh },
    { id: "embed", title: "嵌入与 Serve", body: guideEmbed },
    { id: "security", title: "Serve 安全模型", body: guideSecurity },
    { id: "wire", title: "二进制协议", body: guideWire },
];
