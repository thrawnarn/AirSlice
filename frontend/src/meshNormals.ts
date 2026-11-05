// Shading normals that keep sharp edges sharp. three's computeVertexNormals
// averages over every face at a point, so on a mesh with shared points each
// edge looks rounded: ribs turn soft and long triangles smear.

import * as THREE from "three";

/** Faces meeting at more than this angle show an edge. */
const CREASE_DEGREES = 35;

/**
 * A geometry with normals from `position` + `index`. At each point the faces
 * are grouped by direction, and a point gets one copy per group.
 */
export function creasedGeometry(positions: Float32Array, indices: Uint16Array | Uint32Array): THREE.BufferGeometry {
  const cosCrease = Math.cos(THREE.MathUtils.degToRad(CREASE_DEGREES));
  const faces = indices.length / 3;
  const vertices = positions.length / 3;

  // Face normals: unit direction, and length = twice the area for weighting.
  const unit = new Float32Array(faces * 3);
  const area = new Float32Array(faces);
  for (let f = 0; f < faces; f++) {
    const a = indices[f * 3] * 3;
    const b = indices[f * 3 + 1] * 3;
    const c = indices[f * 3 + 2] * 3;
    const ux = positions[b] - positions[a];
    const uy = positions[b + 1] - positions[a + 1];
    const uz = positions[b + 2] - positions[a + 2];
    const vx = positions[c] - positions[a];
    const vy = positions[c + 1] - positions[a + 1];
    const vz = positions[c + 2] - positions[a + 2];
    const nx = uy * vz - uz * vy;
    const ny = uz * vx - ux * vz;
    const nz = ux * vy - uy * vx;
    const len = Math.hypot(nx, ny, nz);
    area[f] = len;
    if (len > 0) {
      unit[f * 3] = nx / len;
      unit[f * 3 + 1] = ny / len;
      unit[f * 3 + 2] = nz / len;
    }
  }

  // Which corners touch each point, as one flat list.
  const start = new Uint32Array(vertices + 1);
  for (let i = 0; i < indices.length; i++) start[indices[i] + 1]++;
  for (let v = 0; v < vertices; v++) start[v + 1] += start[v];
  const fill = start.slice(0, vertices);
  const corners = new Uint32Array(indices.length);
  for (let i = 0; i < indices.length; i++) corners[fill[indices[i]]++] = i;

  const outIndex = new Uint32Array(indices.length);
  const outPos: number[] = [];
  const outNormal: number[] = [];
  // Each group: running normal sum. Compared against its current direction,
  // so the cost stays low even where hundreds of faces share one point.
  const sums: number[] = [];
  const members: number[][] = [];
  for (let v = 0; v < vertices; v++) {
    sums.length = 0;
    members.length = 0;
    for (let k = start[v]; k < start[v + 1]; k++) {
      const corner = corners[k];
      const f = Math.floor(corner / 3);
      const [x, y, z] = [unit[f * 3], unit[f * 3 + 1], unit[f * 3 + 2]];
      let group = -1;
      for (let g = 0; g < members.length && group < 0; g++) {
        const [sx, sy, sz] = [sums[g * 3], sums[g * 3 + 1], sums[g * 3 + 2]];
        const len = Math.hypot(sx, sy, sz);
        // A face without area has no direction: it joins whatever is there.
        if (area[f] === 0 || len === 0 || (x * sx + y * sy + z * sz) / len >= cosCrease) group = g;
      }
      if (group < 0) {
        group = members.length;
        sums.push(0, 0, 0);
        members.push([]);
      }
      sums[group * 3] += x * area[f];
      sums[group * 3 + 1] += y * area[f];
      sums[group * 3 + 2] += z * area[f];
      members[group].push(corner);
    }
    for (let g = 0; g < members.length; g++) {
      const id = outPos.length / 3;
      const len = Math.hypot(sums[g * 3], sums[g * 3 + 1], sums[g * 3 + 2]) || 1;
      outPos.push(positions[v * 3], positions[v * 3 + 1], positions[v * 3 + 2]);
      outNormal.push(sums[g * 3] / len, sums[g * 3 + 1] / len, sums[g * 3 + 2] / len);
      for (const corner of members[g]) outIndex[corner] = id;
    }
  }

  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(outPos), 3));
  g.setAttribute("normal", new THREE.BufferAttribute(new Float32Array(outNormal), 3));
  g.setIndex(new THREE.BufferAttribute(outIndex, 1));
  return g;
}
