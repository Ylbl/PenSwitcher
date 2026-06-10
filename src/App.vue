<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  BadgePlus,
  ChevronDown,
  ChevronRight,
  Crosshair,
  FolderOpen,
  Keyboard,
  Layers,
  LocateFixed,
  MousePointerClick,
  RefreshCw,
  Search,
  SquareDashedMousePointer,
  Zap,
} from "@lucide/vue";

type ViewName = "processes" | "inspector" | "shortcuts";
type InspectMode = "target-click" | "tree-click";

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface ProcessWindow {
  processId: number;
  title: string;
  hwnd: number;
  className: string;
  bounds?: Rect | null;
}

interface UiNode {
  id: string;
  parentId?: string | null;
  name: string;
  automationId: string;
  controlType: string;
  className: string;
  frameworkId: string;
  processId: number;
  bounds?: Rect | null;
  depth: number;
  hasChildren: boolean;
  childrenLoaded?: boolean;
  expanded?: boolean;
  loading?: boolean;
}

interface DetailRow {
  name: string;
  value: string;
  action?: string | null;
}

interface DetailGroup {
  title: string;
  rows: DetailRow[];
}

interface ElementDetails {
  node: UiNode;
  groups: DetailGroup[];
  supportsInvoke: boolean;
  shortcutEnabled: boolean;
}

interface ShortcutItem {
  id: string;
  process: ProcessWindow;
  node: UiNode;
  hotkey: string;
  enabled: boolean;
  supportsInvoke: boolean;
  status: string;
}

interface PickedElementPayload {
  process: ProcessWindow;
  node: UiNode;
  path: UiNode[];
}

const view = ref<ViewName>("processes");
const dragBtn = ref<HTMLButtonElement | null>(null);
const mode = ref<InspectMode>("tree-click");
const status = ref("就绪");
const processFilter = ref("");
const processes = ref<ProcessWindow[]>([]);
const selectedProcess = ref<ProcessWindow | null>(null);
const rootNodes = ref<UiNode[]>([]);
const flatNodes = computed(() => flattenNodes(rootNodes.value));
const selectedNodeId = ref("");
const selectedNode = computed(() => flatNodes.value.find((node) => node.id === selectedNodeId.value) ?? null);
const visibleNodes = computed(() => {
  const byId = new Map(flatNodes.value.map((node) => [node.id, node]));
  return flatNodes.value.filter((node) => {
    let parentId = node.parentId;
    while (parentId) {
      const parent = byId.get(parentId);
      if (!parent || !parent.expanded) return false;
      parentId = parent.parentId ?? "";
    }
    return true;
  });
});
const details = ref<ElementDetails | null>(null);
const shortcuts = ref<ShortcutItem[]>([]);
const pickingWindow = ref(false);
const elementPickActive = ref(false);
const hotkeyDraft = reactive<Record<string, string>>({});
let windowPickTimer = 0;
let unlistenPicked: UnlistenFn | null = null;
let unlistenHotkey: UnlistenFn | null = null;

const filteredProcesses = computed(() => {
  const q = processFilter.value.trim().toLowerCase();
  if (!q) return processes.value;
  return processes.value.filter((item) =>
    `${item.title} ${item.processId} ${item.className}`.toLowerCase().includes(q),
  );
});

const selectedShortcutIds = computed(() => new Set(shortcuts.value.map((item) => item.id)));
const currentShortcutChecked = computed(() => {
  if (!selectedProcess.value || !selectedNode.value) return false;
  return selectedShortcutIds.value.has(shortcutKey(selectedProcess.value, selectedNode.value));
});
const currentSupportsInvoke = computed(() => details.value?.supportsInvoke ?? false);

onMounted(async () => {
  await Promise.all([refreshProcesses(), refreshShortcuts()]);
  unlistenPicked = await listen<PickedElementPayload>("uia-picked", async (event) => {
    await acceptPickedElement(event.payload);
  });
  unlistenHotkey = await listen<ShortcutItem>("shortcut-invoked", async (event) => {
    status.value = `已触发 ${nodeLabel(event.payload.node)}`;
    await refreshShortcuts();
  });
});

onUnmounted(() => {
  stopWindowPick();
  void invoke("cancel_element_pick");
  void invoke("hide_overlay");
  unlistenPicked?.();
  unlistenHotkey?.();
});

async function refreshProcesses() {
  processes.value = await invoke<ProcessWindow[]>("list_process_windows");
}

async function openInspector(process: ProcessWindow) {
  selectedProcess.value = process;
  view.value = "inspector";
  status.value = `正在读取 ${process.title}`;
  rootNodes.value = await invoke<UiNode[]>("load_tree_root", { process });
  selectedNodeId.value = rootNodes.value[0]?.id ?? "";
  if (selectedNode.value) {
    await selectNode(selectedNode.value, false);
  }
  status.value = "就绪";
}

async function reloadTree() {
  if (!selectedProcess.value) return;
  await openInspector(selectedProcess.value);
}

async function loadChildren(node: UiNode) {
  if (!selectedProcess.value || node.childrenLoaded || node.loading || !node.hasChildren) return;
  node.loading = true;
  try {
    const children = await invoke<UiNode[]>("load_children", {
      process: selectedProcess.value,
      nodeId: node.id,
    });
    insertChildren(node, children);
    node.childrenLoaded = true;
  } finally {
    node.loading = false;
  }
}

async function toggleNode(node: UiNode) {
  if (!node.hasChildren) {
    await selectNode(node);
    return;
  }
  if (!node.expanded) {
    await loadChildren(node);
  }
  node.expanded = !node.expanded;
}

async function selectNode(node: UiNode, drawOverlay = true) {
  selectedNodeId.value = node.id;
  if (!selectedProcess.value) return;
  details.value = await invoke<ElementDetails>("get_element_details", {
    process: selectedProcess.value,
    nodeId: node.id,
  });
  if (drawOverlay && mode.value === "tree-click") {
    await invoke("highlight_element", { process: selectedProcess.value, nodeId: node.id });
  }
}

async function toggleShortcut(checked: boolean) {
  if (!selectedProcess.value || !selectedNode.value) return;
  if (checked && !currentSupportsInvoke.value) {
    status.value = "当前元素无法 Invoke，不能加入快捷操作";
    return;
  }
  shortcuts.value = await invoke<ShortcutItem[]>("set_shortcut_membership", {
    process: selectedProcess.value,
    nodeId: selectedNode.value.id,
    checked,
  });
  if (details.value) {
    details.value.shortcutEnabled = checked;
  }
}

async function refreshShortcuts() {
  shortcuts.value = await invoke<ShortcutItem[]>("list_shortcuts");
  for (const item of shortcuts.value) {
    hotkeyDraft[item.id] = item.hotkey;
  }
}

async function saveHotkey(item: ShortcutItem) {
  const hotkey = (hotkeyDraft[item.id] ?? "").trim();
  shortcuts.value = await invoke<ShortcutItem[]>("set_shortcut_hotkey", { itemId: item.id, hotkey });
  status.value = hotkey ? `已绑定 ${displayHotkey(hotkey)}` : "已清空快捷键";
}

async function removeShortcut(item: ShortcutItem) {
  shortcuts.value = await invoke<ShortcutItem[]>("remove_shortcut", { itemId: item.id });
  delete hotkeyDraft[item.id];
}

async function invokeShortcut(item: ShortcutItem) {
  await invoke("invoke_shortcut", { itemId: item.id });
  await refreshShortcuts();
}

async function switchMode(next: InspectMode) {
  mode.value = next;
  if (next === "target-click") {
    elementPickActive.value = true;
    if (selectedProcess.value) {
      await invoke("start_element_pick", { process: selectedProcess.value });
      status.value = "点击目标软件以定位 UIA 元素";
    }
  } else {
    elementPickActive.value = false;
    await invoke("cancel_element_pick");
    if (selectedProcess.value && selectedNode.value) {
      await invoke("highlight_element", { process: selectedProcess.value, nodeId: selectedNode.value.id });
    }
    status.value = "点击 UIA 树节点以高亮目标元素";
  }
}

async function acceptPickedElement(payload: PickedElementPayload) {
  if (!selectedProcess.value || payload.process.hwnd !== selectedProcess.value.hwnd) return;
  await ensurePath(payload.path);
  selectedNodeId.value = payload.node.id;
  await selectNode(payload.node, false);
  await invoke("highlight_element", { process: selectedProcess.value, nodeId: payload.node.id });
  status.value = `已定位 ${nodeLabel(payload.node)}，可继续点击目标软件`;
}

async function ensurePath(path: UiNode[]) {
  for (let i = 0; i < path.length - 1; i += 1) {
    const existing = flatNodes.value.find((node) => node.id === path[i].id);
    if (!existing) continue;
    if (!existing.childrenLoaded) {
      await loadChildren(existing);
    }
    existing.expanded = true;
  }
}

function beginWindowPick(event: PointerEvent) {
  pickingWindow.value = true;
  (event.currentTarget as HTMLButtonElement)?.setPointerCapture(event.pointerId);
  status.value = "按住并移动到目标窗口，松开后选择";
  windowPickTimer = window.setInterval(async () => {
    if (!pickingWindow.value) return;
    const candidate = await invoke<ProcessWindow | null>("preview_window_under_cursor");
    if (candidate) {
      selectedProcess.value = candidate;
    }
  }, 80);
}

async function finishWindowPick(event: PointerEvent) {
  if (!pickingWindow.value) return;
  stopWindowPick();
  (event.currentTarget as HTMLButtonElement)?.releasePointerCapture(event.pointerId);
  const candidate = await invoke<ProcessWindow | null>("finish_window_pick");
  if (candidate) {
    selectedProcess.value = candidate;
    status.value = `已选中 ${candidate.title}`;
  } else {
    status.value = "未选中窗口";
  }
}

function stopWindowPick() {
  pickingWindow.value = false;
  if (windowPickTimer) {
    window.clearInterval(windowPickTimer);
    windowPickTimer = 0;
  }
}

function insertChildren(parent: UiNode, children: UiNode[]) {
  const list = rootNodes.value;
  const index = list.findIndex((item) => item.id === parent.id);
  if (index < 0) return;
  const descendants = new Set(flattenNodes(children).map((item) => item.id));
  rootNodes.value = list.filter((item) => !descendants.has(item.id));
  rootNodes.value.splice(index + 1, 0, ...children);
}

function flattenNodes(nodes: UiNode[]) {
  return nodes;
}

function nodeLabel(node: UiNode) {
  const name = node.name || "";
  const automationId = node.automationId || "";
  return `${node.controlType} "${name}" ${automationId ? `"${automationId}"` : "\"\""}`;
}

function shortcutKey(process: ProcessWindow, node: UiNode) {
  return `${process.hwnd}:${node.id}`;
}

async function captureHotkey(event: KeyboardEvent, item: ShortcutItem) {
  event.preventDefault();
  event.stopPropagation();
  const key = normalizeKey(event);
  if (!key) return;
  hotkeyDraft[item.id] = key;
  await saveHotkey(item);
}

function normalizeKey(event: KeyboardEvent) {
  const code = event.code;
  if (!code || code === "Unidentified") {
    return "";
  }
  const parts: string[] = [];
  const modifierCodes = new Set(["ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"]);
  const isModifierKey = modifierCodes.has(code);
  if (!isModifierKey) {
    if (event.ctrlKey) parts.push("Ctrl");
    if (event.altKey) parts.push("Alt");
    if (event.shiftKey) parts.push("Shift");
    if (event.metaKey) parts.push("Meta");
  }
  parts.push(code);
  return parts.join("+");
}

function displayHotkey(value: string) {
  return value
    .split("+")
    .map((part) => displayHotkeyPart(part.trim()))
    .filter(Boolean)
    .join("+");
}

function displayHotkeyPart(part: string) {
  if (/^Key[A-Z]$/.test(part)) return part.slice(3);
  if (/^Digit\d$/.test(part)) return part.slice(5);
  if (/^Numpad\d$/.test(part)) return `小键盘${part.slice(6)}`;
  const labels: Record<string, string> = {
    Ctrl: "Ctrl",
    Control: "Ctrl",
    Alt: "Alt",
    Shift: "Shift",
    Meta: "Win",
    ControlLeft: "左Ctrl",
    ControlRight: "右Ctrl",
    ShiftLeft: "左Shift",
    ShiftRight: "右Shift",
    AltLeft: "左Alt",
    AltRight: "右Alt",
    MetaLeft: "左Win",
    MetaRight: "右Win",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    Space: "Space",
    Escape: "Esc",
    Backspace: "Backspace",
    Delete: "Delete",
    Enter: "Enter",
    Tab: "Tab",
    PageUp: "PageUp",
    PageDown: "PageDown",
    Insert: "Insert",
    Home: "Home",
    End: "End",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backquote: "`",
    NumpadAdd: "小键盘+",
    NumpadSubtract: "小键盘-",
    NumpadMultiply: "小键盘*",
    NumpadDivide: "小键盘/",
    NumpadDecimal: "小键盘.",
    NumpadEnter: "小键盘Enter",
  };
  return labels[part] ?? part;
}
</script>

<template>
  <div class="h-full bg-[#17191c] text-[13px] text-slate-100">
    <header class="flex h-12 items-center justify-between border-b border-[#3b4047] bg-[#202225] px-3">
      <div class="flex items-center gap-2">
        <div class="flex h-8 w-8 items-center justify-center border border-[#4a5058] bg-[#282b30] text-sky-300">
          <Layers :size="17" />
        </div>
        <div>
          <div class="text-sm font-semibold leading-4">UIA 指示器</div>
          <div class="text-[11px] text-slate-400">{{ status }}</div>
        </div>
      </div>
      <div class="flex items-center gap-1">
        <button
          v-if="view !== 'processes'"
          class="toolbar-button"
          title="返回进程"
          @click="view = 'processes'; invoke('hide_overlay')"
        >
          <ArrowLeft :size="16" />
        </button>
        <button class="toolbar-button" title="进程列表" @click="view = 'processes'">
          <FolderOpen :size="16" />
        </button>
        <button class="toolbar-button" title="快捷操作" @click="view = 'shortcuts'; refreshShortcuts()">
          <Keyboard :size="16" />
        </button>
      </div>
    </header>

    <main v-if="view === 'processes'" class="grid h-[calc(100%-3rem)] grid-cols-[340px_1fr] bg-[#181a1d]">
      <section class="border-r border-[#373c43] bg-[#1f2226] p-4">
        <div class="mb-4 flex items-center justify-between">
          <div>
            <h1 class="text-lg font-semibold">选择进程</h1>
            <p class="text-xs text-slate-400">选择一个窗口后进入 UIA 检查</p>
          </div>
          <button class="toolbar-button h-9 w-9" title="刷新" @click="refreshProcesses">
            <RefreshCw :size="17" />
          </button>
        </div>
        <div class="relative mb-3">
          <Search class="absolute left-2 top-2.5 text-slate-500" :size="15" />
          <input
            v-model="processFilter"
            class="h-9 w-full border border-[#464c55] bg-[#151719] pl-8 pr-3 text-sm outline-none focus:border-sky-500"
            placeholder="搜索标题、进程 ID 或类名"
          />
        </div>
        <button
          ref="dragBtn"
          class="mb-4 flex h-10 w-full items-center justify-center gap-2 border border-sky-700 bg-sky-950/50 text-sky-100 hover:bg-sky-900/60"
          @pointerdown="beginWindowPick"
          @pointerup="finishWindowPick"
          @pointercancel="finishWindowPick"
        >
          <SquareDashedMousePointer :size="17" />
          按住拖到目标窗口
        </button>
        <div class="text-xs text-slate-400">
          当前选中：<span class="text-slate-100">{{ selectedProcess?.title || "无" }}</span>
        </div>
        <button
          class="mt-3 h-9 w-full border border-[#4a5058] bg-[#282c32] px-4 text-slate-100 hover:border-sky-600 hover:bg-sky-950/50 disabled:cursor-not-allowed disabled:opacity-40"
          :disabled="!selectedProcess"
          @click="selectedProcess && openInspector(selectedProcess)"
        >
          打开检查器
        </button>
      </section>

      <section class="flex min-w-0 flex-col">
        <div class="flex h-8 items-center border-b border-[#373c43] bg-[#202327] px-3 text-xs text-slate-300">
          Windows
        </div>
        <div class="flex-1 overflow-auto p-2">
          <button
            v-for="process in filteredProcesses"
            :key="process.hwnd"
            class="grid w-full grid-cols-[1fr_90px_120px] items-center gap-3 border border-transparent px-3 py-2 text-left hover:border-[#4a5563] hover:bg-[#242930]"
            :class="selectedProcess?.hwnd === process.hwnd ? 'border-sky-700 bg-sky-950/50' : ''"
            @click="selectedProcess = process"
            @dblclick="openInspector(process)"
          >
            <span class="truncate font-medium">{{ process.title || "(Untitled)" }}</span>
            <span class="text-xs text-slate-400">PID {{ process.processId }}</span>
            <span class="truncate text-xs text-slate-500">{{ process.className || "Window" }}</span>
          </button>
        </div>
      </section>
    </main>

    <main v-else-if="view === 'inspector'" class="flex h-[calc(100%-3rem)] flex-col bg-[#1b1d20]">
      <div class="flex h-11 items-center gap-2 border-b border-[#3b4047] bg-[#202225] px-2">
        <button class="toolbar-button" title="刷新" @click="reloadTree">
          <RefreshCw :size="16" />
        </button>
        <div class="mx-1 h-6 w-px bg-[#444a52]"></div>
        <button
          class="mode-button"
          :class="mode === 'target-click' ? 'mode-active' : ''"
          title="点击目标软件"
          @click="switchMode('target-click')"
        >
          <MousePointerClick :size="16" />
          点击目标软件
        </button>
        <button
          class="mode-button"
          :class="mode === 'tree-click' ? 'mode-active' : ''"
          title="点击 UIA 树"
          @click="switchMode('tree-click')"
        >
          <LocateFixed :size="16" />
          点击UIA树
        </button>
        <div class="ml-auto truncate text-xs text-slate-400">
          {{ selectedProcess?.title }}
        </div>
      </div>

      <div class="grid min-h-0 flex-1 grid-cols-[minmax(380px,50%)_1fr]">
        <section class="min-w-0 border-r border-[#424850]">
          <div class="panel-title">Elements</div>
          <div class="h-[calc(100%-1.5rem)] overflow-auto bg-[#24272b] py-1">
            <div
              v-for="node in visibleNodes"
              :key="node.id"
              class="tree-row"
              :class="selectedNodeId === node.id ? 'tree-row-selected' : ''"
              :style="{ paddingLeft: `${8 + node.depth * 16}px` }"
              @click="selectNode(node)"
              @dblclick.stop="toggleNode(node)"
            >
              <button class="mr-1 flex h-4 w-4 items-center justify-center" @click.stop="toggleNode(node)">
                <ChevronDown v-if="node.hasChildren && node.expanded" :size="14" />
                <ChevronRight v-else-if="node.hasChildren" :size="14" />
              </button>
              <span class="truncate">{{ nodeLabel(node) }}</span>
            </div>
          </div>
        </section>

        <section class="min-w-0">
          <div class="panel-title">Details</div>
          <div class="h-[calc(100%-1.5rem)] overflow-auto bg-[#1f2225]">
            <div class="flex h-9 items-center justify-between border-b border-[#454b54] bg-[#25282d] px-2">
              <label class="flex items-center gap-2 text-xs text-slate-100">
                <input
                  type="checkbox"
                  class="h-4 w-4 accent-sky-500"
                  :checked="currentShortcutChecked"
                  :disabled="!selectedNode || !currentSupportsInvoke"
                  @change="toggleShortcut(($event.target as HTMLInputElement).checked)"
                />
                {{ currentSupportsInvoke ? "加入快捷操作" : "无法 Invoke" }}
              </label>
              <button class="toolbar-button h-7 w-7" title="查看快捷操作" @click="view = 'shortcuts'; refreshShortcuts()">
                <Zap :size="15" />
              </button>
            </div>

            <template v-if="details">
              <div v-for="group in details.groups" :key="group.title" class="detail-group">
                <div class="detail-group-title">
                  <Crosshair :size="13" />
                  {{ group.title }}
                </div>
                <div v-for="row in group.rows" :key="`${group.title}-${row.name}`" class="detail-row">
                  <div class="truncate px-2">{{ row.name }}</div>
                  <div class="truncate px-2 font-medium text-white">{{ row.value }}</div>
                </div>
              </div>
            </template>
          </div>
        </section>
      </div>
    </main>

    <main v-else class="h-[calc(100%-3rem)] bg-[#1b1d20]">
      <div class="flex h-11 items-center justify-between border-b border-[#3b4047] bg-[#202225] px-3">
        <div class="flex items-center gap-2 font-semibold">
          <Keyboard :size="17" />
          快捷操作
        </div>
        <button class="toolbar-button" title="刷新" @click="refreshShortcuts">
          <RefreshCw :size="16" />
        </button>
      </div>
      <div class="h-[calc(100%-2.75rem)] overflow-auto p-3">
        <div class="grid grid-cols-[1fr_220px_130px_110px] border border-[#454b54] bg-[#202327] text-xs font-semibold text-slate-300">
          <div class="px-3 py-2">元素</div>
          <div class="px-3 py-2">快捷键</div>
          <div class="px-3 py-2">状态</div>
          <div class="px-3 py-2 text-right">操作</div>
        </div>
        <div
          v-for="item in shortcuts"
          :key="item.id"
          class="grid grid-cols-[1fr_220px_130px_110px] items-center border-x border-b border-[#454b54] bg-[#24272b] text-xs"
        >
          <div class="min-w-0 px-3 py-2">
            <div class="truncate text-slate-100">{{ nodeLabel(item.node) }}</div>
            <div class="truncate text-slate-500">{{ item.process.title }} · PID {{ item.process.processId }}</div>
          </div>
          <div class="px-3 py-2">
            <input
              :value="displayHotkey(hotkeyDraft[item.id] ?? '')"
              class="h-8 w-full border border-[#4a5058] bg-[#151719] px-2 outline-none focus:border-sky-500"
              placeholder="点击后按快捷键"
              readonly
              @keydown="captureHotkey($event, item)"
              @paste.prevent
            />
          </div>
          <div class="px-3 py-2" :class="item.supportsInvoke ? 'text-emerald-300' : 'text-amber-300'">
            {{ item.status }}
          </div>
          <div class="flex justify-end gap-1 px-3 py-2">
            <button class="small-button" :disabled="!item.supportsInvoke" @click="invokeShortcut(item)">
              <BadgePlus :size="14" />
            </button>
            <button class="small-button" @click="removeShortcut(item)">删除</button>
          </div>
        </div>
      </div>
    </main>
  </div>
</template>

<style scoped>
.toolbar-button {
  display: inline-flex;
  height: 32px;
  width: 32px;
  align-items: center;
  justify-content: center;
  border: 1px solid #4a5058;
  background: #292c31;
  color: #e6edf7;
}

.toolbar-button:hover,
.small-button:hover {
  border-color: #60a5fa;
  background: #263449;
}

.mode-button {
  display: inline-flex;
  height: 32px;
  align-items: center;
  gap: 6px;
  border: 1px solid #4a5058;
  background: #292c31;
  padding: 0 10px;
  color: #d7deea;
}

.mode-active {
  border-color: #0284c7;
  background: #0c4a6e;
  color: #f0f9ff;
}

.panel-title {
  height: 24px;
  border-bottom: 1px solid #424850;
  background: #1d2023;
  padding: 2px 4px;
  font-weight: 600;
}

.tree-row {
  display: flex;
  height: 20px;
  cursor: default;
  align-items: center;
  border: 1px solid transparent;
  color: #f4f7fb;
  white-space: nowrap;
}

.tree-row:hover {
  background: #2d333b;
}

.tree-row-selected {
  border-color: #0891b2;
  background: #075985;
}

.detail-group {
  border-bottom: 1px solid #424850;
}

.detail-group-title {
  display: flex;
  height: 24px;
  align-items: center;
  gap: 5px;
  border-bottom: 1px solid #454b54;
  background: #292d32;
  padding: 0 5px;
  font-weight: 700;
}

.detail-row {
  display: grid;
  min-height: 20px;
  grid-template-columns: minmax(130px, 28%) 1fr;
  align-items: center;
  background: #24272b;
}

.detail-row:nth-child(odd) {
  background: #565a60;
}

.small-button {
  display: inline-flex;
  height: 28px;
  align-items: center;
  justify-content: center;
  border: 1px solid #4a5058;
  background: #292c31;
  padding: 0 8px;
  color: #e6edf7;
}

.small-button:disabled {
  cursor: not-allowed;
  opacity: 0.4;
}
</style>
