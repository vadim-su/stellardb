import ForceGraph2D, { type ForceGraphMethods as ForceGraph2DMethods } from "react-force-graph-2d";
import type { ForceGraphMethods as ForceGraph3DMethods } from "react-force-graph-3d";
import { Box, Maximize2, Minimize2, Network, ScanSearch, Square } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { StationQueryResult } from "../lib/station-api";
import {
  extractStationGraph,
  type StationGraphLink,
  type StationGraphNode,
} from "../lib/station-graph";

type Projection = "2d" | "3d";
type ForceGraph3DComponent = typeof import("react-force-graph-3d")["default"];
type ThreeRuntime = typeof import("three");

const GRAPH_PROJECTION_KEY = "stellardb:station:graph-projection";
const MAX_RENDERED_NODES = 400;

function endpointId(endpoint: string | StationGraphNode) {
  return typeof endpoint === "string" ? endpoint : endpoint.id;
}

function sampleGraph(
  data: { nodes: StationGraphNode[]; links: StationGraphLink[] },
  limit = MAX_RENDERED_NODES,
) {
  if (data.nodes.length <= limit) return data;

  const degree = new Map<string, number>();
  for (const link of data.links) {
    const source = endpointId(link.source);
    const target = endpointId(link.target);
    degree.set(source, (degree.get(source) ?? 0) + 1);
    degree.set(target, (degree.get(target) ?? 0) + 1);
  }

  let nodes: StationGraphNode[];
  if (degree.size > 0) {
    nodes = [...data.nodes]
      .sort((left, right) => (degree.get(right.id) ?? 0) - (degree.get(left.id) ?? 0))
      .slice(0, limit);
  } else {
    nodes = Array.from({ length: limit }, (_, index) => (
      data.nodes[Math.floor(index * data.nodes.length / limit)]!
    ));
  }

  const visibleIds = new Set(nodes.map((node) => node.id));
  const links = data.links.filter((link) => (
    visibleIds.has(endpointId(link.source)) && visibleIds.has(endpointId(link.target))
  ));
  return { nodes, links };
}

export function StationGraph({ result }: { result: StationQueryResult }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const graph2dRef = useRef<ForceGraph2DMethods<StationGraphNode, StationGraphLink> | undefined>(undefined);
  const graph3dRef = useRef<ForceGraph3DMethods<StationGraphNode, StationGraphLink> | undefined>(undefined);
  const [dimensions, setDimensions] = useState({ width: 760, height: 300 });
  const [selected, setSelected] = useState<StationGraphNode | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [ForceGraph3D, setForceGraph3D] = useState<ForceGraph3DComponent | null>(null);
  const [Three, setThree] = useState<ThreeRuntime | null>(null);
  const [projection, setProjection] = useState<Projection>(() => {
    try {
      return localStorage.getItem(GRAPH_PROJECTION_KEY) === "3d" ? "3d" : "2d";
    } catch {
      return "2d";
    }
  });
  const data = useMemo(() => extractStationGraph(result), [result]);
  const visibleData = useMemo(() => sampleGraph(data), [data]);
  const collections = useMemo(() => {
    const colors = new Map<string, string>();
    for (const node of data.nodes) {
      if (!colors.has(node.collection)) colors.set(node.collection, node.color);
    }
    return [...colors.entries()].map(([name, color]) => ({ name, color }));
  }, [data]);
  const sampled = visibleData.nodes.length < data.nodes.length;
  const dense = visibleData.nodes.length > 120;

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const update = () => setDimensions({ width: element.clientWidth, height: element.clientHeight });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [fullscreen]);

  useEffect(() => {
    try {
      localStorage.setItem(GRAPH_PROJECTION_KEY, projection);
    } catch {
      // The graph still works when browser storage is unavailable.
    }
  }, [projection]);

  useEffect(() => {
    if (projection !== "3d" || (ForceGraph3D && Three)) return;
    let cancelled = false;
    void Promise.all([import("react-force-graph-3d"), import("three")]).then(([graphModule, threeModule]) => {
      if (cancelled) return;
      setForceGraph3D(() => graphModule.default);
      setThree(threeModule);
    });
    return () => { cancelled = true; };
  }, [ForceGraph3D, projection, Three]);

  useEffect(() => setSelected(null), [result]);

  useEffect(() => {
    if (!fullscreen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFullscreen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [fullscreen]);

  if (data.nodes.length === 0) {
    return (
      <div className="graph-empty">
        <Network size={22} />
        <strong>No graph data detected</strong>
        <span>Return document IDs or traversal results to enable Graph view.</span>
      </div>
    );
  }

  function isSelectedLink(link: StationGraphLink) {
    if (!selected) return false;
    return endpointId(link.source) === selected.id || endpointId(link.target) === selected.id;
  }

  function drawNode(node: StationGraphNode, context: CanvasRenderingContext2D, scale: number) {
    const isSelected = selected?.id === node.id;
    const radius = isSelected ? 6 : dense ? 2.4 : 4;
    context.beginPath();
    context.arc(node.x ?? 0, node.y ?? 0, radius, 0, Math.PI * 2);
    context.fillStyle = node.color;
    context.globalAlpha = isSelected ? 1 : dense ? 0.72 : 0.9;
    context.shadowColor = node.color;
    context.shadowBlur = isSelected ? 14 : dense ? 0 : 6;
    context.fill();
    context.globalAlpha = 1;
    context.shadowBlur = 0;

    if (!isSelected && (dense || scale < 1.1)) return;
    const fontSize = Math.max(10 / scale, 2);
    context.font = `${fontSize}px 'DM Mono', monospace`;
    context.textAlign = "center";
    context.textBaseline = "top";
    context.fillStyle = "rgba(232,237,246,.8)";
    context.fillText(node.label, node.x ?? 0, (node.y ?? 0) + radius + 2);
  }

  function fitGraph() {
    if (projection === "3d") graph3dRef.current?.zoomToFit(350, 48);
    else graph2dRef.current?.zoomToFit(350, 48);
  }

  function create3dLabel(node: StationGraphNode) {
    const three = Three;
    if (!three) throw new Error("3D runtime is not loaded");
    const group = new three.Group();
    if (dense && selected?.id !== node.id) return group;

    const label = node.label.length > 30 ? `${node.label.slice(0, 29)}…` : node.label;
    const canvas = document.createElement("canvas");
    let context = canvas.getContext("2d");
    if (!context) return group;
    context.font = "500 28px 'DM Mono', monospace";
    canvas.width = Math.ceil(context.measureText(label).width) + 32;
    canvas.height = 52;
    context = canvas.getContext("2d");
    if (!context) return group;
    context.fillStyle = "rgba(9, 13, 20, .82)";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.strokeStyle = `${node.color}88`;
    context.lineWidth = 2;
    context.strokeRect(1, 1, canvas.width - 2, canvas.height - 2);
    context.font = "500 28px 'DM Mono', monospace";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillStyle = "#e8edf6";
    context.fillText(label, canvas.width / 2, canvas.height / 2 + 1);

    const texture = new three.CanvasTexture(canvas);
    texture.colorSpace = three.SRGBColorSpace;
    const sprite = new three.Sprite(new three.SpriteMaterial({
      map: texture,
      transparent: true,
      depthTest: false,
    }));
    const height = 2.4;
    sprite.scale.set(height * canvas.width / canvas.height, height, 1);
    sprite.position.set(0, 3.4, 0);
    group.add(sprite);
    return group;
  }

  const nodeCount = sampled
    ? `${visibleData.nodes.length} / ${data.nodes.length} nodes`
    : `${data.nodes.length} nodes`;
  const edgeCount = sampled && visibleData.links.length !== data.links.length
    ? `${visibleData.links.length} / ${data.links.length} edges`
    : `${data.links.length} edges`;

  return (
    <div className={`station-graph ${fullscreen ? "is-fullscreen" : ""}`} ref={containerRef}>
      <div className="graph-toolbar">
        <span title={sampled ? `Showing a representative sample of ${visibleData.nodes.length} from ${data.nodes.length} nodes` : undefined}>
          <Network size={13} /> {nodeCount} · {edgeCount}{sampled && <small>sampled</small>}
        </span>
        <div className="graph-projection" role="group" aria-label="Graph projection">
          <button className={projection === "2d" ? "is-active" : ""} onClick={() => setProjection("2d")} title="2D projection" aria-pressed={projection === "2d"}><Square size={12} /> 2D</button>
          <button className={projection === "3d" ? "is-active" : ""} onClick={() => setProjection("3d")} title="3D projection" aria-pressed={projection === "3d"}><Box size={12} /> 3D</button>
        </div>
        <button onClick={fitGraph} title="Fit graph" aria-label="Fit graph"><ScanSearch size={14} /></button>
        <button onClick={() => setFullscreen((value) => !value)} title={fullscreen ? "Exit fullscreen" : "Fullscreen"} aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}>
          {fullscreen ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
        </button>
      </div>

      {projection === "2d" ? (
        <ForceGraph2D
          ref={graph2dRef}
          graphData={visibleData}
          width={dimensions.width}
          height={dimensions.height}
          backgroundColor="rgba(0,0,0,0)"
          nodeCanvasObject={drawNode}
          nodeCanvasObjectMode={() => "replace"}
          nodeLabel={(node) => (node as StationGraphNode).id}
          linkColor={(link) => isSelectedLink(link as StationGraphLink) ? "rgba(185,155,255,.72)" : "rgba(132,145,170,.24)"}
          linkWidth={(link) => isSelectedLink(link as StationGraphLink) ? 2 : dense ? 0.5 : 1}
          linkDirectionalArrowLength={dense ? 0 : 4}
          linkDirectionalArrowRelPos={1}
          linkDirectionalArrowColor={() => "rgba(185,155,255,.55)"}
          linkCurvature={0.08}
          onNodeClick={(node) => setSelected(node as StationGraphNode)}
          onBackgroundClick={() => setSelected(null)}
          cooldownTime={dense ? 900 : 1800}
        />
      ) : ForceGraph3D && Three ? (
        <ForceGraph3D
          ref={graph3dRef}
          graphData={visibleData}
          width={dimensions.width}
          height={dimensions.height}
          backgroundColor="rgba(0,0,0,0)"
          showNavInfo={false}
          nodeColor={(node) => (node as StationGraphNode).color}
          nodeVal={(node) => selected?.id === (node as StationGraphNode).id ? 5 : dense ? 1.4 : 2.4}
          nodeOpacity={dense ? 0.72 : 0.92}
          nodeResolution={dense ? 8 : 16}
          nodeLabel={(node) => (node as StationGraphNode).id}
          nodeThreeObject={(node: unknown) => create3dLabel(node as StationGraphNode)}
          nodeThreeObjectExtend={true}
          linkColor={(link) => isSelectedLink(link as StationGraphLink) ? "#b99bff" : "#596274"}
          linkWidth={(link) => isSelectedLink(link as StationGraphLink) ? 1.4 : dense ? 0.35 : 0.7}
          linkOpacity={dense ? 0.2 : 0.45}
          linkDirectionalArrowLength={dense ? 0 : 2}
          linkDirectionalArrowRelPos={1}
          onNodeClick={(node) => setSelected(node as StationGraphNode)}
          onBackgroundClick={() => setSelected(null)}
          cooldownTime={dense ? 1200 : 2200}
        />
      ) : (
        <div className="graph-loading">Loading 3D projection…</div>
      )}

      <div className="graph-legend" aria-label="Graph legend">
        {collections.slice(0, 6).map((collection) => (
          <span key={collection.name} title={`Nodes from ${collection.name}`}>
            <i style={{ background: collection.color }} /> {collection.name}
          </span>
        ))}
        {collections.length > 6 && <span>+{collections.length - 6} collections</span>}
        {data.links.length > 0 && <span><i className="is-link" /> relation</span>}
      </div>

      {selected && (
        <aside className="graph-node-detail">
          <button onClick={() => setSelected(null)} aria-label="Close node details">×</button>
          <span style={{ background: selected.color }} />
          <small>{selected.collection}</small>
          <strong>{selected.id}</strong>
          <dl>
            {Object.entries(selected.fields).slice(0, 8).map(([key, value]) => (
              <div key={key}><dt>{key}</dt><dd>{typeof value === "object" ? JSON.stringify(value) : String(value ?? "null")}</dd></div>
            ))}
          </dl>
        </aside>
      )}
    </div>
  );
}
