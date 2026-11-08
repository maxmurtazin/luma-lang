---
title: One-page syntax reference
weight: 2
---

**Draft 001 · Syntax frontend available; execution unimplemented.** A compact one-A4-page syntax goal; physical pagination has not been verified. Full rules: [specification]({{< relref "specification" >}}).

~~~text
# Values and assignments; indentation is insignificant
count = 3
name = "Luma"
active = true
numbers = [1, 2, 3]
numbers[0] = 4

fn square(x)
    return x * x
end
result = numbers -> map(square) -> sum

status = match count
    > 30: "Hot"
    > 20: "Warm"
    *: "Cold"
end

fn double(x)
    return x * 2
end
fn finished(x)
    return x >= 100
end
result = 10 -> repeat(double, finished)
squares = numbers -> parallel(square)

object Building
    height = 25
    fn volume(area)
        return area * height
    end
end
volume = Building.volume(120)

# math.luma
pub fn twice(x)
    return x * 2
end
# main.luma (separate file)
use math
answer = 4 -> math.twice
~~~

Blocks: fn / object / match … end. Match arms use : and final *. -> inserts its input as the first argument; = assigns. Operators: + - * / % ^, == != < > <= >=, and or not; calls (), arrays/indexes [], qualification .; # comments. No braces, significant indentation, let/var, classes, overloads, async, for/while. Static inference, by-value arguments, ordered synchronous parallel results; callbacks require conservative effect checking.

