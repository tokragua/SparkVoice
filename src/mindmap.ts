import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface AppSettings {
    llm_mind_map_enabled: boolean;
    llm_node_cap: number;
}

interface DBGraphNode {
    id: number;
    name: string;
    entity_type: string;
}

interface DBGraphEdge {
    id: number;
    source_id: number;
    target_id: number;
    relation_type: string;
    context: string;
}

interface DBMindMapGraph {
    nodes: DBGraphNode[];
    edges: DBGraphEdge[];
}

// ── Types ────────────────────────────────────────────────────────────────────

interface LogEntry {
    timestamp: string;
    text: string;
    duration: number;
}

interface MindNode {
    id: number;
    label: string;      // truncated (~40 chars)
    fullText: string;   // full sentence
    keywords: Set<string>;
    x: number;
    y: number;
    vx: number;
    vy: number;
    isRoot: boolean;    // topic cluster root
    topic: string;      // dominant keyword / topic label
    alpha: number;      // entrance animation 0→1
    radius: number;
    w: number;
    h: number;
    degree: number;
}

interface Edge {
    a: number;
    b: number;
    strength: number;
}

// ── Stop words ───────────────────────────────────────────────────────────────

const STOP_WORDS = new Set([
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
    "by", "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
    "this", "that", "these", "those", "it", "its", "i", "you", "he", "she", "we", "they",
    "my", "your", "his", "her", "our", "their", "me", "him", "us", "them", "not", "no",
    "so", "if", "as", "then", "than", "up", "out", "there", "here", "about", "into",
    "just", "also", "very", "some", "all", "more", "when", "what", "which", "who", "how",
    "any", "each", "only", "other", "over", "such", "than", "too", "now", "new", "get",
    "got", "its", "yes", "after", "before", "same", "most",
]);

// ── Date helpers ─────────────────────────────────────────────────────────────

function toDateInputValue(d: Date): string {
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
}

// ── NLP helpers ──────────────────────────────────────────────────────────────

function extractKeywords(text: string): Set<string> {
    return new Set(
        text
            .toLowerCase()
            .replace(/[^a-z0-9\s'-]/g, " ")
            .split(/\s+/)
            .filter((w) => w.length >= 4 && !STOP_WORDS.has(w))
    );
}

function splitSentences(text: string): string[] {
    // Split on sentence-ending punctuation, keep non-trivial sentences
    return text
        .split(/(?<=[.?!])\s+/)
        .map((s) => s.trim())
        .filter((s) => s.length > 10 && s.split(" ").length > 2);
}

function truncate(text: string, max = 42): string {
    if (text.length <= max) return text;
    return text.slice(0, max - 1) + "…";
}

function keywordOverlap(a: Set<string>, b: Set<string>): number {
    let count = 0;
    for (const k of a) if (b.has(k)) count++;
    return count;
}

function dominantKeyword(kws: Set<string>, freq: Map<string, number>): string {
    let best = "";
    let bestScore = -1;
    for (const k of kws) {
        const score = freq.get(k) ?? 0;
        if (score > bestScore) { bestScore = score; best = k; }
    }
    return best;
}

// ── Build graph ───────────────────────────────────────────────────────────────

function buildGraph(entries: LogEntry[]): { nodes: MindNode[]; edges: Edge[] } {
    // 1. Collect sentences
    const allSentences: { text: string; keywords: Set<string> }[] = [];
    for (const e of entries) {
        for (const sent of splitSentences(e.text)) {
            const keywords = extractKeywords(sent);
            if (keywords.size > 0) {
                allSentences.push({ text: sent, keywords });
            }
        }
    }

    // Deduplicate near-identical sentences (simple === check)
    const seen = new Set<string>();
    const unique = allSentences.filter((s) => {
        if (seen.has(s.text.trim().toLowerCase())) return false;
        seen.add(s.text.trim().toLowerCase());
        return true;
    });

    // Cap at 120 nodes for performance
    const capped = unique.slice(0, 120);

    // 2. Build keyword frequency map
    const kwFreq = new Map<string, number>();
    for (const s of capped) {
        for (const k of s.keywords) kwFreq.set(k, (kwFreq.get(k) ?? 0) + 1);
    }

    // 3. Create nodes, spreading in a rough circle initially
    const nodes: MindNode[] = capped.map((s, i) => {
        const angle = (i / capped.length) * Math.PI * 2;
        const r = 300 + Math.random() * 200;
        const topic = dominantKeyword(s.keywords, kwFreq);
        return {
            id: i,
            label: truncate(s.text),
            fullText: s.text,
            keywords: s.keywords,
            x: Math.cos(angle) * r,
            y: Math.sin(angle) * r,
            vx: 0, vy: 0,
            isRoot: false,
            topic,
            alpha: 0,
            radius: 0,
            w: 0, h: 0,
            degree: 0,
        };
    });

    // 4. Build edges for pairs with ≥ 2 shared keywords
    const edges: Edge[] = [];
    for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
            const overlap = keywordOverlap(nodes[i].keywords, nodes[j].keywords);
            if (overlap >= 2) {
                edges.push({ a: i, b: j, strength: Math.min(overlap / 5, 1) });
            }
        }
    }

    // 5. Mark nodes as cluster roots (highest-degree nodes per topic)
    const topicLeaders = new Map<string, number>(); // topic → node index
    const degreeMap = new Map<number, number>();
    for (const e of edges) {
        degreeMap.set(e.a, (degreeMap.get(e.a) ?? 0) + 1);
        degreeMap.set(e.b, (degreeMap.get(e.b) ?? 0) + 1);
    }
    for (const n of nodes) {
        const deg = degreeMap.get(n.id) ?? 0;
        const existing = topicLeaders.get(n.topic);
        if (existing === undefined || deg > (degreeMap.get(existing) ?? 0)) {
            topicLeaders.set(n.topic, n.id);
        }
    }
    for (const [, idx] of topicLeaders) {
        if ((degreeMap.get(idx) ?? 0) >= 2) nodes[idx].isRoot = true;
    }

    return { nodes, edges };
}

// ── Force-directed layout ────────────────────────────────────────────────────

async function runLayout(
    nodes: MindNode[],
    edges: Edge[],
    iterations = 300
): Promise<void> {
    const REPEL = 8000;
    const ATTRACT = 0.04;
    const DAMPING = 0.85;
    const CENTER_PULL = 0.003;

    // Scale down iterations for large graphs to avoid multi-second hangs
    const adaptiveIters = Math.max(30, Math.min(iterations, Math.floor(36000 / nodes.length)));
    const YIELD_EVERY = 10; // yield to browser event loop every N iterations

    for (let iter = 0; iter < adaptiveIters; iter++) {
        // Repel
        for (let i = 0; i < nodes.length; i++) {
            for (let j = i + 1; j < nodes.length; j++) {
                const dx = nodes[i].x - nodes[j].x;
                const dy = nodes[i].y - nodes[j].y;
                const dist2 = dx * dx + dy * dy + 1;
                const force = REPEL / dist2;
                const nx = dx / Math.sqrt(dist2);
                const ny = dy / Math.sqrt(dist2);
                nodes[i].vx += nx * force;
                nodes[i].vy += ny * force;
                nodes[j].vx -= nx * force;
                nodes[j].vy -= ny * force;
            }
        }
        // Attract edges
        for (const e of edges) {
            const a = nodes[e.a];
            const b = nodes[e.b];
            const dx = b.x - a.x;
            const dy = b.y - a.y;
            const f = ATTRACT * e.strength;
            a.vx += dx * f; a.vy += dy * f;
            b.vx -= dx * f; b.vy -= dy * f;
        }
        // Center pull + dampen
        for (const n of nodes) {
            n.vx += -n.x * CENTER_PULL;
            n.vy += -n.y * CENTER_PULL;
            n.vx *= DAMPING;
            n.vy *= DAMPING;
            n.x += n.vx;
            n.y += n.vy;
        }
        // Yield to browser so UI stays responsive
        if ((iter + 1) % YIELD_EVERY === 0) {
            await new Promise<void>(resolve => setTimeout(resolve, 0));
        }
    }
}

// Rectangle-aware separation — runs AFTER node sizes are known
function separateNodes(nodes: MindNode[], iterations = 120): void {
    const PAD = 14; // minimum gap between node edges
    for (let iter = 0; iter < iterations; iter++) {
        let anyOverlap = false;
        for (let i = 0; i < nodes.length; i++) {
            for (let j = i + 1; j < nodes.length; j++) {
                const a = nodes[i];
                const b = nodes[j];
                const minDX = (a.w + b.w) / 2 + PAD;
                const minDY = (a.h + b.h) / 2 + PAD;
                const dx = b.x - a.x;
                const dy = b.y - a.y;
                const absDX = Math.abs(dx);
                const absDY = Math.abs(dy);
                if (absDX < minDX && absDY < minDY) {
                    anyOverlap = true;
                    // Push apart on the axis of least penetration
                    const overlapX = minDX - absDX;
                    const overlapY = minDY - absDY;
                    if (overlapX < overlapY) {
                        const push = overlapX / 2 + 0.5;
                        const dir = dx >= 0 ? 1 : -1;
                        a.x -= dir * push;
                        b.x += dir * push;
                    } else {
                        const push = overlapY / 2 + 0.5;
                        const dir = dy >= 0 ? 1 : -1;
                        a.y -= dir * push;
                        b.y += dir * push;
                    }
                }
            }
        }
        if (!anyOverlap) break; // converged early
    }
}

// ── Canvas renderer ──────────────────────────────────────────────────────────

const COLORS = {
    nodeBgFrom: "#131c2e",
    nodeBgTo: "#0e1525",
    nodeBorder: "rgba(99,102,241,0.35)",
    nodeHover: "rgba(99,102,241,0.85)",
    rootFrom: "#312e81",
    rootTo: "#1e1b4b",
    rootBorder: "rgba(167,139,250,0.7)",
    edge: "rgba(99,102,241,0.22)",
    edgeHover: "rgba(167,139,250,0.6)",
    text: "#e2e8f0",
    textRoot: "#c4b5fd",
    label: "#94a3b8",
};

const NODE_PAD_X = 18;
const NODE_PAD_Y = 12;
const NODE_RADIUS = 10;
const FONT_SIZE = 12.5;
const ROOT_FONT_SIZE = 13;

class MindMapRenderer {
    private canvas: HTMLCanvasElement;
    private ctx: CanvasRenderingContext2D;
    private nodes: MindNode[] = [];
    private edges: Edge[] = [];
    private panX = 0;
    private panY = 0;
    private scale = 1;
    private isDragging = false;
    private lastMX = 0;
    private lastMY = 0;
    private hoveredNode: MindNode | null = null;
    private selectedNode: MindNode | null = null;
    private connectedNodeIds: Set<number> = new Set();
    private mouseDownPos = { x: 0, y: 0 };
    private animStart = 0;
    private animRunning = false;
    private raf = 0;
    private tooltip: HTMLElement;
    private tooltipText: HTMLElement;
    private statsBar: HTMLElement;
    private wrap: HTMLElement;
    private maxDegree = 1;

    constructor() {
        this.canvas = document.getElementById("map-canvas") as HTMLCanvasElement;
        this.ctx = this.canvas.getContext("2d")!;
        this.tooltip = document.getElementById("tooltip")!;
        this.tooltipText = document.getElementById("tooltip-text")!;
        this.statsBar = document.getElementById("stats-bar")!;
        this.wrap = document.getElementById("canvas-wrap")!;
        this.resize();
        window.addEventListener("resize", () => this.resize());
        this.canvas.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
        this.canvas.addEventListener("mousedown", (e) => this.onMouseDown(e));
        window.addEventListener("mousemove", (e) => this.onMouseMove(e));
        window.addEventListener("mouseup", (e) => this.onMouseUp(e));
    }

    resize() {
        const dpr = window.devicePixelRatio || 1;
        const w = this.wrap.clientWidth;
        const h = this.wrap.clientHeight;
        this.canvas.width = w * dpr;
        this.canvas.height = h * dpr;
        this.canvas.style.width = w + "px";
        this.canvas.style.height = h + "px";
        this.ctx.scale(dpr, dpr);
        if (this.nodes.length) this.drawFrame();
    }

    load(nodes: MindNode[], edges: Edge[]) {
        this.nodes = nodes;
        this.edges = edges;
        this.maxDegree = Math.max(1, ...nodes.map(n => n.degree));
        // Compute node sizes
        const ctx = this.ctx;
        for (const n of nodes) {
            const fs = n.isRoot ? ROOT_FONT_SIZE : FONT_SIZE;
            ctx.font = `${n.isRoot ? 600 : 500} ${fs}px Inter, system-ui, sans-serif`;
            const tw = ctx.measureText(n.label).width;
            n.w = tw + NODE_PAD_X * 2;
            n.h = fs + NODE_PAD_Y * 2;
            n.radius = n.isRoot ? 12 : NODE_RADIUS;
        }
        // Resolve any remaining overlaps now that we have exact sizes
        separateNodes(nodes);
        // Center the view
        this.panX = this.wrap.clientWidth / 2;
        this.panY = this.wrap.clientHeight / 2;
        this.scale = 1;
        // Reset selection
        this.selectedNode = null;
        this.connectedNodeIds = new Set();
        this.animStart = performance.now();
        this.animRunning = true;
        cancelAnimationFrame(this.raf);
        this.drawLoop();
    }

    private drawLoop() {
        this.raf = requestAnimationFrame(() => this.drawLoop());
        const t = (performance.now() - this.animStart) / 600; // 0→1 over 600ms
        const done = t >= 1;
        for (const n of this.nodes) {
            n.alpha = done ? 1 : this.easeOut(Math.min(t + n.id * 0.015, 1));
        }
        this.drawFrame();
        if (done && !this.hoveredNode && !this.isDragging) {
            this.animRunning = false;
            cancelAnimationFrame(this.raf);
        }
    }

    private easeOut(x: number): number {
        return 1 - Math.pow(1 - x, 3);
    }

    drawFrame() {
        const { ctx, canvas } = this;
        const dpr = window.devicePixelRatio || 1;
        const W = canvas.width / dpr;
        const H = canvas.height / dpr;
        ctx.clearRect(0, 0, W, H);

        ctx.save();
        ctx.translate(this.panX, this.panY);
        ctx.scale(this.scale, this.scale);

        // Draw edges first
        for (const e of this.edges) {
            const a = this.nodes[e.a];
            const b = this.nodes[e.b];
            const alpha = Math.min(a.alpha, b.alpha);
            if (alpha <= 0) continue;
            const isHovered = a === this.hoveredNode || b === this.hoveredNode;
            const isSelected = this.selectedNode !== null &&
                (a === this.selectedNode || b === this.selectedNode);
            const hasSelection = this.selectedNode !== null;

            ctx.save();
            if (hasSelection) {
                ctx.globalAlpha = alpha * (isSelected ? 0.85 : 0.06);
                ctx.strokeStyle = isSelected ? "rgba(167,139,250,0.9)" : COLORS.edge;
                ctx.lineWidth = isSelected ? 2 : 1;
            } else {
                ctx.globalAlpha = alpha * (isHovered ? 0.7 : 0.35);
                ctx.strokeStyle = isHovered ? COLORS.edgeHover : COLORS.edge;
                ctx.lineWidth = isHovered ? 1.5 : 1;
            }
            // Cubic bezier between node centers
            const mx = (a.x + b.x) / 2;
            const cx1 = a.x + (mx - a.x) * 0.5;
            const cy1 = a.y;
            const cx2 = b.x - (b.x - mx) * 0.5;
            const cy2 = b.y;
            ctx.beginPath();
            ctx.moveTo(a.x, a.y);
            ctx.bezierCurveTo(cx1, cy1, cx2, cy2, b.x, b.y);
            ctx.stroke();
            ctx.restore();
        }

        // Draw nodes
        for (const n of this.nodes) {
            if (n.alpha <= 0) continue;
            this.drawNode(n);
        }

        ctx.restore();
    }

    private drawNode(n: MindNode) {
        const { ctx } = this;
        const isHovered = n === this.hoveredNode;
        const isSelected = n === this.selectedNode;
        const hasSelection = this.selectedNode !== null;
        const isConnected = this.connectedNodeIds.has(n.id);

        // Dim nodes not involved in current selection
        const dimmed = hasSelection && !isSelected && !isConnected;

        const x = n.x - n.w / 2;
        const y = n.y - n.h / 2;

        ctx.save();
        ctx.globalAlpha = n.alpha * (dimmed ? 0.18 : 1);

        // Shadow / glow
        if (isSelected) {
            ctx.shadowColor = "rgba(167,139,250,0.8)";
            ctx.shadowBlur = 28;
        } else if (isConnected && hasSelection) {
            ctx.shadowColor = "rgba(99,102,241,0.5)";
            ctx.shadowBlur = 16;
        } else if (isHovered) {
            ctx.shadowColor = "rgba(99,102,241,0.6)";
            ctx.shadowBlur = 20;
        }

        // Filled rounded rect
        const grad = ctx.createLinearGradient(x, y, x, y + n.h);
        const ratio = this.maxDegree > 0 ? (n.degree / this.maxDegree) : 0;

        if (n.isRoot) {
            grad.addColorStop(0, COLORS.rootFrom);
            grad.addColorStop(1, COLORS.rootTo);
        } else if (ratio > 0) {
            const h = 244 + (ratio * 116); // 244 (Indigo) -> 360 (Red)
            const l1 = 20 + ratio * 15; // lighter when red (up to 35%)
            const l2 = 34 + ratio * 15; // lighter when red (up to 49%)
            grad.addColorStop(0, `hsl(${h}, 60%, ${l1}%)`);
            grad.addColorStop(1, `hsl(${h}, 60%, ${l2}%)`);
        } else {
            grad.addColorStop(0, COLORS.nodeBgFrom);
            grad.addColorStop(1, COLORS.nodeBgTo);
        }
        ctx.fillStyle = grad;
        ctx.beginPath();
        this.roundRect(ctx, x, y, n.w, n.h, n.radius);
        ctx.fill();

        // Border
        ctx.shadowBlur = 0;

        let borderColor = n.isRoot ? COLORS.rootBorder : COLORS.nodeBorder;
        if (!n.isRoot && ratio > 0) {
            const h = 244 + (ratio * 116);
            borderColor = `hsla(${h}, 80%, 65%, 0.5)`;
        }

        ctx.strokeStyle = isSelected
            ? "rgba(167,139,250,1)"
            : (isConnected && hasSelection)
                ? "rgba(129,140,248,0.9)"
                : isHovered
                    ? COLORS.nodeHover
                    : borderColor;
        ctx.lineWidth = isSelected ? 2 : (isConnected && hasSelection) ? 1.5 : isHovered ? 1.5 : 1;
        ctx.beginPath();
        this.roundRect(ctx, x, y, n.w, n.h, n.radius);
        ctx.stroke();

        // Text
        const fs = n.isRoot ? ROOT_FONT_SIZE : FONT_SIZE;
        ctx.font = `${n.isRoot ? 600 : 500} ${fs}px Inter, system-ui, sans-serif`;
        ctx.fillStyle = isSelected
            ? "#fff"
            : (isConnected && hasSelection)
                ? "#c4b5fd"
                : n.isRoot
                    ? COLORS.textRoot
                    : (dimmed ? "#475569" : COLORS.text);
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(n.label, n.x, n.y);

        ctx.restore();
    }

    private roundRect(
        ctx: CanvasRenderingContext2D,
        x: number, y: number, w: number, h: number, r: number
    ) {
        ctx.moveTo(x + r, y);
        ctx.lineTo(x + w - r, y);
        ctx.arcTo(x + w, y, x + w, y + r, r);
        ctx.lineTo(x + w, y + h - r);
        ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
        ctx.lineTo(x + r, y + h);
        ctx.arcTo(x, y + h, x, y + h - r, r);
        ctx.lineTo(x, y + r);
        ctx.arcTo(x, y, x + r, y, r);
        ctx.closePath();
    }

    private worldPos(mx: number, my: number): { x: number; y: number } {
        return {
            x: (mx - this.panX) / this.scale,
            y: (my - this.panY) / this.scale,
        };
    }

    private hitTest(wx: number, wy: number): MindNode | null {
        for (const n of this.nodes) {
            if (
                wx >= n.x - n.w / 2 &&
                wx <= n.x + n.w / 2 &&
                wy >= n.y - n.h / 2 &&
                wy <= n.y + n.h / 2
            ) {
                return n;
            }
        }
        return null;
    }

    private onWheel(e: WheelEvent) {
        e.preventDefault();
        const factor = e.deltaY < 0 ? 1.1 : 0.9;
        const rect = this.canvas.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        // Zoom toward cursor
        this.panX = mx - (mx - this.panX) * factor;
        this.panY = my - (my - this.panY) * factor;
        this.scale *= factor;
        this.scale = Math.max(0.15, Math.min(4, this.scale));
        this.drawFrame();
    }

    private onMouseDown(e: MouseEvent) {
        if (e.button !== 0) return;
        this.isDragging = true;
        this.lastMX = e.clientX;
        this.lastMY = e.clientY;
        this.mouseDownPos = { x: e.clientX, y: e.clientY };
        this.wrap.classList.add("grabbing");
    }

    private onMouseMove(e: MouseEvent) {
        // Pan
        if (this.isDragging) {
            this.panX += e.clientX - this.lastMX;
            this.panY += e.clientY - this.lastMY;
            this.lastMX = e.clientX;
            this.lastMY = e.clientY;
            this.drawFrame();
            return;
        }

        // Hit test for hover / tooltip
        if (!this.nodes.length) return;
        const rect = this.canvas.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        const { x: wx, y: wy } = this.worldPos(mx, my);
        const hit = this.hitTest(wx, wy);

        if (hit !== this.hoveredNode) {
            this.hoveredNode = hit;
            this.canvas.style.cursor = hit ? "pointer" : "default";
            if (!this.animRunning) {
                cancelAnimationFrame(this.raf);
                this.drawFrame();
            }
        }

        if (hit) {
            this.tooltipText.textContent = hit.fullText;
            const tx = Math.min(e.clientX + 14, window.innerWidth - 360);
            const ty = Math.min(e.clientY + 14, window.innerHeight - 100);
            this.tooltip.style.left = tx + "px";
            this.tooltip.style.top = ty + "px";
            this.tooltip.classList.add("visible");
        } else {
            this.tooltip.classList.remove("visible");
        }
    }

    private onMouseUp(e: MouseEvent) {
        this.isDragging = false;
        this.wrap.classList.remove("grabbing");

        // Only treat as a click if mouse barely moved (not a pan)
        const dx = e.clientX - this.mouseDownPos.x;
        const dy = e.clientY - this.mouseDownPos.y;
        if (Math.sqrt(dx * dx + dy * dy) > 4) return;

        // Hit-test in world space
        const rect = this.canvas.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        const { x: wx, y: wy } = this.worldPos(mx, my);
        const hit = this.hitTest(wx, wy);

        if (hit && hit !== this.selectedNode) {
            // Select this node
            this.selectedNode = hit;
            this.connectedNodeIds = new Set(
                this.edges
                    .filter((ed) => ed.a === hit.id || ed.b === hit.id)
                    .flatMap((ed) => [ed.a, ed.b])
            );
        } else {
            // Deselect (click same node or empty space)
            this.selectedNode = null;
            this.connectedNodeIds = new Set();
        }

        this.drawFrame();
    }

    setStats(nodes: number, edges: number, entries: number) {
        this.statsBar.innerHTML =
            `<span>${nodes}</span> nodes &nbsp;·&nbsp; <span>${edges}</span> connections &nbsp;·&nbsp; <span>${entries}</span> log entries`;
        this.statsBar.classList.add("visible");
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main() {
    const renderer = new MindMapRenderer();
    const emptyState = document.getElementById("empty-state")!;
    const btnCreate = document.getElementById("btn-create") as HTMLButtonElement;
    const btnFull = document.getElementById("btn-fullscreen") as HTMLButtonElement;
    const dateFrom = document.getElementById("date-from") as HTMLInputElement;
    const dateTo = document.getElementById("date-to") as HTMLInputElement;

    // Check settings to see which mode we're in
    let settings: AppSettings | null = null;
    try {
        settings = await invoke<AppSettings>("get_settings");
    } catch (e) {
        console.warn("Could not load settings in mind map:", e);
    }
    const isLLMEnabled = settings?.llm_mind_map_enabled ?? false;
    const llmNodeCap = settings?.llm_node_cap ?? 4000;

    // Default range: last 7 days
    const today = new Date();
    const weekAgo = new Date(today);
    weekAgo.setDate(today.getDate() - 7);
    dateTo.value = toDateInputValue(today);
    dateFrom.value = toDateInputValue(weekAgo);

    // Fullscreen toggle
    btnFull.addEventListener("click", async () => {
        const win = getCurrentWindow();
        const isFs = await win.isFullscreen();
        await win.setFullscreen(!isFs);
        btnFull.textContent = isFs ? "⛶" : "⛶";
    });

    // Create mind map
    btnCreate.addEventListener("click", async () => {
        const from = dateFrom.value;
        const to = dateTo.value;
        if (!from || !to || from > to) {
            dateFrom.style.borderColor = "#f87171";
            dateTo.style.borderColor = "#f87171";
            setTimeout(() => {
                dateFrom.style.borderColor = "";
                dateTo.style.borderColor = "";
            }, 1500);
            return;
        }

        btnCreate.textContent = "Building…";
        btnCreate.classList.add("loading");

        try {
            let nodes: MindNode[] = [];
            let edges: Edge[] = [];
            let entryCount = 0;

            if (isLLMEnabled) {
                // === LLM GRAPH MODE ===
                const dbGraph = await invoke<DBMindMapGraph>("get_mind_map_graph", {
                    limit: 10000,
                    keyword: null,
                    fromDate: from,
                    toDate: to
                });

                if (!dbGraph || dbGraph.nodes.length === 0) {
                    emptyState.querySelector("p")!.innerHTML =
                        `No extracted entities found yet.<br/>Ensure the Knowledge Graph is enabled and you have active transcriptions.`;
                    emptyState.classList.remove("hidden");
                    return;
                }

                // Cap nodes for layout performance (configurable in settings)
                const cappedNodes = dbGraph.nodes.slice(0, llmNodeCap);

                // Map DB IDs to array indices for edges
                const idToIndex = new Map<number, number>();
                cappedNodes.forEach((dbn, i) => idToIndex.set(dbn.id, i));

                // Convert DB graph to visual MindNode/Edge format
                nodes = cappedNodes.map((dbn, i) => {
                    const angle = (i / cappedNodes.length) * Math.PI * 2;
                    const r = 300 + Math.random() * 200;
                    return {
                        id: i,
                        label: dbn.name,
                        fullText: `${dbn.name} (${dbn.entity_type})`,
                        keywords: new Set([dbn.entity_type]), // Use type as topic
                        x: Math.cos(angle) * r,
                        y: Math.sin(angle) * r,
                        vx: 0, vy: 0,
                        isRoot: false,
                        topic: dbn.entity_type,
                        alpha: 0,
                        radius: 0,
                        w: 0, h: 0,
                        degree: 0,
                    };
                });

                // Deduplicate edges by undirected node pair to avoid visual clutter
                // from the same relationship appearing across multiple log files
                const edgeSet = new Set<string>();
                for (const dbe of dbGraph.edges) {
                    const idxA = idToIndex.get(dbe.source_id);
                    const idxB = idToIndex.get(dbe.target_id);
                    if (idxA !== undefined && idxB !== undefined) {
                        const key = `${Math.min(idxA, idxB)}-${Math.max(idxA, idxB)}`;
                        if (!edgeSet.has(key)) {
                            edgeSet.add(key);
                            edges.push({ a: idxA, b: idxB, strength: 1.0 });
                        }
                    }
                }
                entryCount = edges.length; // Approximate "entries" as edges plotted
                emptyState.classList.add("hidden");

            } else {
                // === LEGACY KEYWORD OVERLAP MODE ===
                const entries = await invoke<LogEntry[]>("get_transcription_logs", {
                    fromDate: from,
                    toDate: to,
                });

                if (!entries || entries.length === 0) {
                    emptyState.querySelector("p")!.innerHTML =
                        `No transcription logs found between <strong>${from}</strong> and <strong>${to}</strong>.<br/>Try a wider date range.`;
                    emptyState.classList.remove("hidden");
                    return;
                }

                emptyState.classList.add("hidden");

                // Build graph
                const graph = buildGraph(entries);
                nodes = graph.nodes;
                edges = graph.edges;
                entryCount = entries.length;

                if (nodes.length === 0) {
                    emptyState.querySelector("p")!.innerHTML =
                        `Logs found but no meaningful sentences could be extracted.<br/>Try a wider date range.`;
                    emptyState.classList.remove("hidden");
                    return;
                }
            }

            // Calculate degrees
            for (const ed of edges) {
                nodes[ed.a].degree++;
                nodes[ed.b].degree++;
            }

            // Run force directed layout and render
            await runLayout(nodes, edges, 300);
            renderer.load(nodes, edges);
            renderer.setStats(nodes.length, edges.length, entryCount);
        } catch (err) {
            console.error("Mind map error:", err);
            const errText = String(err).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
            emptyState.querySelector("p")!.innerHTML =
                `Failed to load logs: <strong>${errText}</strong>`;
            emptyState.classList.remove("hidden");
        } finally {
            btnCreate.textContent = "Create";
            btnCreate.classList.remove("loading");
        }
    });
}

main();
