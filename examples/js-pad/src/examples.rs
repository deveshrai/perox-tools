//! Example programs, from the toolbar's Examples list.

pub const NAMES: [&str; 5] = ["Hello", "Fibonacci", "Classes", "Sorting", "Promises"];

pub fn get(name: &str) -> Option<&'static str> {
    Some(match name {
        "Hello" => HELLO,
        "Fibonacci" => FIBONACCI,
        "Classes" => CLASSES,
        "Sorting" => SORTING,
        "Promises" => PROMISES,
        _ => return None,
    })
}

const HELLO: &str = r#"// Welcome to JS Pad. Press Run (⌘R / Ctrl+R) and watch the console.
// Type in the console below to try things out: it shares this file's scope.

function greet(name) {
  const hour = new Date().getHours();
  const part = hour < 12 ? "morning" : hour < 18 ? "afternoon" : "evening";
  return `Good ${part}, ${name}!`;
}

const names = ["Ada", "Grace", "Linus"];
for (const n of names) {
  console.log(greet(n));
}

console.info(`${names.length} people greeted`);
"#;

const FIBONACCI: &str = r#"// Three ways to the Fibonacci numbers.

function fibRecursive(n) {
  return n < 2 ? n : fibRecursive(n - 1) + fibRecursive(n - 2);
}

function fibLoop(n) {
  let [a, b] = [0, 1];
  for (let i = 0; i < n; i++) {
    [a, b] = [b, a + b];
  }
  return a;
}

function* fibonacci() {
  let [a, b] = [0n, 1n];
  while (true) {
    yield a;
    [a, b] = [b, a + b];
  }
}

console.log("recursive:", fibRecursive(20));
console.log("loop:", fibLoop(50));

const first = [];
for (const f of fibonacci()) {
  if (first.length === 12) break;
  first.push(f.toString());
}
console.log("generator:", first.join(", "));
"#;

const CLASSES: &str = r#"// Classes, getters, inheritance and static members.

class Shape {
  static count = 0;
  constructor(name) {
    this.name = name;
    Shape.count++;
  }
  get area() {
    return 0;
  }
  describe() {
    return `${this.name} with area ${this.area.toFixed(2)}`;
  }
}

class Circle extends Shape {
  constructor(r) {
    super("circle");
    this.r = r;
  }
  get area() {
    return Math.PI * this.r ** 2;
  }
}

class Rect extends Shape {
  constructor(w, h) {
    super("rectangle");
    Object.assign(this, { w, h });
  }
  get area() {
    return this.w * this.h;
  }
}

const shapes = [new Circle(1.5), new Rect(3, 4), new Circle(0.5)];
shapes.sort((a, b) => b.area - a.area).forEach(s => console.log(s.describe()));
console.log("shapes made:", Shape.count);
"#;

const SORTING: &str = r#"// Sorting an array of records, and a quicksort of our own.

const people = [
  { name: "Mei", age: 34, city: "Taipei" },
  { name: "Ola", age: 27, city: "Oslo" },
  { name: "Ravi", age: 41, city: "Pune" },
  { name: "Ana", age: 27, city: "Lisbon" },
];

const byAgeThenName = [...people].sort(
  (a, b) => a.age - b.age || a.name.localeCompare(b.name)
);
console.log(byAgeThenName.map(p => `${p.name} (${p.age})`).join(", "));

function quicksort([pivot, ...rest]) {
  if (pivot === undefined) return [];
  const smaller = rest.filter(x => x < pivot);
  const larger = rest.filter(x => x >= pivot);
  return [...quicksort(smaller), pivot, ...quicksort(larger)];
}

const numbers = Array.from({ length: 12 }, (_, i) => (i * 37) % 23);
console.log("before:", numbers);
console.log("after: ", quicksort(numbers));
console.warn("Sorted", numbers.length, "numbers");
"#;

const PROMISES: &str = r#"// async functions and promises: the console shows the order things happen in.

const wait = value => Promise.resolve(value);

async function brew(drink) {
  console.log(`grinding for ${drink}…`);
  await wait();
  console.log(`pouring ${drink}`);
  return `${drink} ready`;
}

console.log("start");
Promise.all([brew("espresso"), brew("latte")]).then(results => {
  console.info(results.join(" and "));
});

async function fails() {
  await wait();
  throw new Error("out of milk");
}
fails().catch(e => console.error("caught:", e.message));
console.log("end of the script (the rest comes after)");
"#;
