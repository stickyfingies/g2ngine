let two = 1 + 1;
let four = two + "2";

say("Hello from demo.js");

function greet(name) {
  return "Hello, " + name + "!";
}

function add([a, b]) {
  return parseInt(a) + parseInt(b);
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

four;
