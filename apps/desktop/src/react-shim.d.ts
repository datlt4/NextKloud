// Chỉ dùng React để mount Excalidraw; không cài @types/react cho nhẹ.
declare module "react" {
  export function createElement(type: any, props?: any, ...children: any[]): any;
}
declare module "react-dom/client" {
  export interface Root {
    render(node: any): void;
    unmount(): void;
  }
  export function createRoot(el: Element): Root;
}
