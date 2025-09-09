let two = 1 + 1;
let four = two + "2";

say("Hello from demo.js");
say(JSON.stringify(glMatrix));

function greet(name) {
  return "Hello, " + name + "!";
}

function add([a, b]) {
  return parseInt(a) + parseInt(b);
}

function makeInstance() {
  return {
    position: {
      x: 0,
      y: 0,
      z: 0,
    },
    rotation: {
      v: {
        x: 0,
        y: 0,
        z: 0,
      },
      s: 0,
    },
  };
}

function getInfo() {
  return "This is demo.js running";
}

function processGameData(input) {
  say(JSON.stringify(input));
}

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

let data = new Float32Array([0.0, 1.0, 2.5, -3.14, 42.0]);
data_fn(data);

four;
