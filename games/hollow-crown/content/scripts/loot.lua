-- Class equipment, scored upgrades, and derived combat attributes.
local L={}
local rarity={'Worn','Tempered','Enchanted','Relic'}
local colors={'#879995','#c3a773','#79a8bd','#b498d3'}
local names={
 knight={weapon={'Arming Sword','Bearded Axe','Runic Longsword','Kingsbane'},armor={'Mail Hauberk','Guard Plate','Runic Cuirass','Saints\' Plate'}},
 ranger={weapon={'Yew Bow','Hunting Crossbow','Moonwood Bow','Starfall'},armor={'Leather Jerkin','Scale Vest','Elven Mail','Shadowweave'}},
 mage={weapon={'Ash Staff','Cinder Wand','Frost Sceptre','Astral Branch'},armor={'Travel Robe','Warded Silk','Runespun Robe','Oracle\'s Mantle'}}
}
function L.starter(class)
 local gear={}
 for _,slot in ipairs({'weapon','armor','boots','charm'}) do gear[slot]={name=slot=='weapon' and names[class].weapon[1] or slot=='armor' and names[class].armor[1] or slot=='boots' and 'Travel Boots' or 'Plain Talisman',slot=slot,tier=0,score=0,color=colors[1],class=class,stats={}} end
 return gear
end
function L.roll(rng,rank,hint,slotHint)
 local class=hint or ({'knight','ranger','mage'})[rng(1,3)]
 local slot=slotHint or ({'weapon','armor','boots','charm'})[rng(1,4)]
 local tier=math.min(4,math.max(1,rank+rng(-1,1)))
 local stats={};local kind='';local base
 if slot=='weapon' then
  stats.damage=tier*3+rng(0,2);stats.hit=math.floor(tier/2);stats.crit=tier
  kind=class=='knight' and (rng(1,2)==1 and 'axe' or 'sword') or class=='ranger' and (rng(1,2)==1 and 'crossbow' or 'bow') or 'staff'
  base=names[class].weapon[tier]
  if class=='knight' and kind=='axe' then base=({'Hewing Axe','War Axe','Runic Bearded Axe','Kingsbane Axe'})[tier] end
  if class=='ranger' then base=kind=='crossbow' and ({'Hunting Crossbow','Steel Crossbow','Moonsteel Crossbow','Nightfall Crossbow'})[tier] or ({'Yew Bow','Recurve Bow','Moonwood Bow','Starfall'})[tier] end
 elseif slot=='armor' then stats.ac=math.ceil(tier/2);stats.hp=tier*4;stats.resist=tier*.025;base=names[class].armor[tier]
 elseif slot=='boots' then stats.speed=tier*.12;stats.cooldown=tier*.025;stats.ac=tier>=3 and 1 or 0;base=({'Scout Boots','Iron Greaves','Windstep Boots','Ghoststride'})[tier]
 else stats.mana=tier*6;stats.regen=tier*.06;stats.resist=tier*.018;stats.hp=tier*2;base=({'Amber Charm','Moonstone Pendant','Wardstone','Saints\' Tear'})[tier] end
 local score=(stats.damage or 0)*3+(stats.hit or 0)*4+(stats.ac or 0)*6+(stats.hp or 0)*.3+(stats.speed or 0)*8+(stats.cooldown or 0)*60+(stats.regen or 0)*8+(stats.resist or 0)*35+(stats.mana or 0)*.1
 return {name=base,slot=slot,tier=tier,rarity=rarity[tier],color=colors[tier],class=class,kind=kind,score=score,stats=stats,value=8+tier*11}
end
function L.derive(h)
 h.maxhp=h.basehp;h.ac=h.baseac;h.damage=0;h.hit=4;h.speed=2.7;h.cooldown=1;h.maxmana=65;h.regen=.35;h.resist=0;h.crit=0
 for _,item in pairs(h.gear) do local s=item.stats
  h.maxhp=h.maxhp+(s.hp or 0);h.ac=h.ac+(s.ac or 0);h.damage=h.damage+(s.damage or 0);h.hit=h.hit+(s.hit or 0);h.speed=h.speed+(s.speed or 0);h.cooldown=h.cooldown-(s.cooldown or 0);h.maxmana=h.maxmana+(s.mana or 0);h.regen=h.regen+(s.regen or 0);h.resist=h.resist+(s.resist or 0);h.crit=h.crit+(s.crit or 0)
 end
 if h.hp then h.hp=math.min(h.hp,h.maxhp) end
 h.mana=math.min(h.mana or h.maxmana,h.maxmana)
end
function L.equip(h,item)
 if item.class~=h.class or item.score<=h.gear[item.slot].score then return false end
 h.gear[item.slot]=item;L.derive(h);return true
end
return L
