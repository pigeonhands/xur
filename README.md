# Xur

Experimental language designed to be used in a repl  for very quick and hacky data manipulation.

Inspired by pyhon and ruby with a little bt of haskell mixed in.

Everything in `Xur` is a function, and all functions can be made partial.

For example, these are all equivilent:
```
to_int("FF", 16)
"FF" to_int 16
to_int(,16)("FF")
"FF".to_int(16)
(,16).to_int("FF") #WIP
```
The parenthesies `(..)` is called a parameter set. Paraeter sets expand to fill a functions arguments with its contents. So a functon can be defined with many parameters, but only called with a max of 2. A parameer set will fill in the rest of the parameters.

A function will only take 1 or 2 arguments. If a parameter set is directly after a function, that will be its only parameter.
```
to_int("FF", 16)
```
If there is no parameter set, it will take the value before it as its first argument, and the value after as its second argument.
```
"FF" to_int 16
```

There are a few special characters that can only be used as complete function names (such as `.`, `+`, `-`, `@` etc.). 

 For example `.` is the full name of a function and it can never be used inside an identifier name. Because of this, there dosent need to be any whitespace when using it as in inline function, like is needed with `to_int` in example 2.


The `.` funtction is just a normal function that converts `a.b` into `b(a)`.

Like haskell, pasing in only some of the arguments to a function is perfectly fine. Instead of receving the return value of the function, you will recieve a partial function.

```
to_int      # returns fn(num:str, base:num) -> num
to_int("FF") # returns fn(base:num) -> num
to_int(,16) # returns fn(num:str) -> num
```

So, for example, mapping `to_int` to a list of integers would  not need a new function or a closure
```
["AA", "BB", "CC"].map(to_int(,16))
```


## Ideas / plans
---
### Functions
Like the idea behind '`.`', i would like **everything** that can possably be a funciton, to be a function. Including the function definition.
```
fn(to_int, (s, base), {
    ...
})
```

`fn` would be a function that takes in an 'identifer' 'param set' and a 'block' and would wrap the block in a lambda that did something like this python-esque pseudo-code

```
def fn(id, params, block):
    f = create_function(block, enviroment={
        s:    params.pop()
        base: params.pop()
    })
    set(id, f)
```

### Type associated functions

Add `->` function wich would act similarly to `.` except it would do some light name mangling.
For example, these woukd be exaclty equivilent
```
"FF"->to_int(16)
__number__to_int("FF", 16)
```
and these defitions would be exactly equivilent
```
fn(__number__to_int, (s, base), {
    ...
})

zn(number, {
    fn(t_int, (s, base), {
       ...
    })
})
```