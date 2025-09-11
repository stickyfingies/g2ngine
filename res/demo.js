const { vec3, quat, mat4 } = glMatrix;

// Init system

function makeInstances() {
  const SPACE_BETWEEN = 3;
  const NUM_INSTANCES_PER_ROW = 30;
  const total_instances = NUM_INSTANCES_PER_ROW * NUM_INSTANCES_PER_ROW;

  // Interleaved format: [pos_x, pos_y, pos_z, rot_x, rot_y, rot_z, rot_w, ...]
  // 7 floats per instance
  const instanceData = new Float32Array(7 * total_instances);

  const displacement = vec3.fromValues(
    NUM_INSTANCES_PER_ROW * 0.5,
    0,
    NUM_INSTANCES_PER_ROW * 0.5,
  );

  let dataIndex = 0;

  for (let x = 0; x < NUM_INSTANCES_PER_ROW; x++) {
    for (let z = 0; z < NUM_INSTANCES_PER_ROW; z++) {
      const position = vec3.multiply(
        vec3.create(),
        vec3.subtract(vec3.create(), vec3.fromValues(x, 0, z), displacement),
        vec3.fromValues(SPACE_BETWEEN, SPACE_BETWEEN, SPACE_BETWEEN),
      );

      let rotation = quat.create();
      if (vec3.length(position) === 0) {
        quat.setAxisAngle(rotation, vec3.fromValues(0, 1, 0), 0);
      } else {
        let position_norm = vec3.normalize(vec3.create(), position);
        quat.setAxisAngle(rotation, position_norm, Math.PI / 4);
      }

      // Pack into interleaved format for fast Rust processing
      instanceData[dataIndex + 0] = position[0]; // pos_x
      instanceData[dataIndex + 1] = position[1]; // pos_y
      instanceData[dataIndex + 2] = position[2]; // pos_z
      instanceData[dataIndex + 3] = rotation[0]; // rot_x
      instanceData[dataIndex + 4] = rotation[1]; // rot_y
      instanceData[dataIndex + 5] = rotation[2]; // rot_z
      instanceData[dataIndex + 6] = rotation[3]; // rot_w

      dataIndex += 7;
    }
  }

  return instanceData;
}

// let data = new Float32Array([0.0, 1.0, 2.5, -3.14, 42.0]);
// data_fn(data);

// Update loop

let color = [Math.random(), Math.random(), Math.random(), 1.0];

function update() {
  if (Math.random() < 0.01) {
    color[0] = Math.random();
  }
  if (Math.random() < 0.01) {
    color[1] = Math.random();
  }
  if (Math.random() < 0.01) {
    color[2] = Math.random();
  }
  return color;
}
