app-title = Vitrine de Day
counter-value = { $count ->
    [one] { $count } clic
   *[other] { $count } clics
}
decrement = −
increment = +
name-placeholder = Votre nom
greeting = Bonjour, { $name } !
volume-label = Volume
progress-label = Progression
busy-label = Occupé
flavor-label = Parfum
history-title = Historique
history-entry = le compteur est passé à { $value }
nav-controls = Contrôles
nav-menus = Menus
nav-text = Texte
nav-battery = Batterie
nav-sensors = Capteurs
nav-clipboard = Presse-papiers
nav-network = Réseau
nav-media = Média
nav-pickers = Sélecteurs
nav-compose = Composition
nav-files = Fichiers
nav-tabs = Onglets
nav-stack = Pile
nav-list = Liste
nav-webview = Vue Web
nav-lottie = Lottie
nav-about = À propos

shapes-kinds = Types
shapes-transform = Transformation
shapes-angle = Angle

picker-shared-caption = Les trois styles sont liés au même signal de sélection — modifiez-en un, les autres suivent.
picker-segmented = Segmenté
picker-menu = Menu
picker-inline = Aligné

compose-caption = Pièces de pure composition — sans code natif, sans fonctionnalités cargo, sur tous les backends gratuitement.
compose-rating-label = Note en étoiles
compose-rating-count = Étoiles sélectionnées :
compose-rating-placeholder = 1–5
compose-card-title = Surface réutilisable
compose-card-body = Marge + arrière-plan + coins arrondis, appliqués comme Modificateur.
compose-plain-btn = Simple
compose-styled-btn = Rempli
compose-env-value = Teinté par l'accent fourni
list-add = Ajouter 100
list-caption = { $count } lignes — seules les cellules visibles sont créées

webview-url-hint = Saisir une URL
webview-go = Aller
webview-back = Précédent
webview-forward = Suivant
webview-stop = Arrêter
webview-reload = Recharger

lottie-caption = Une animation Lottie native, fournie en JSON (lottie-ios / lottie-android)
lottie-speed = Vitesse
stack-root-body = Une vraie pile push/pop. Son chemin est un signal de l'application.
stack-push = Empiler un détail
stack-detail-title = Niveau { $depth }
stack-detail-body = Empilé sur le chemin. Le bouton retour natif réécrit le dépilement.
stack-item-title = Élément { $id }
stack-link-42 = Ouvrir item-42 avec un indice (route absolue)
stack-param-hint = Ouvert avec l'indice : {$hint}
tab-one = Aperçu
tab-two = Détails
tab-three = Réglages
tab-one-body = L'onglet aperçu. Chaque onglet conserve son propre état.
tab-two-body = L'onglet détails, sélectionné par sa clé de route.
tab-three-body = L'onglet réglages. Les liens profonds et dayscript choisissent les onglets par clé.
about-text = Une application native multiplateforme construite avec day.
nav-modals = Fenêtres
modal-alert = Afficher l'alerte
modal-confirm = Confirmer
modal-delete = Supprimer…
modal-sheet = Choisir un parfum
modal-prompt = Saisir le nom
alert-title = Avis
alert-body = Vos modifications ont été enregistrées.
ok = OK
confirm-title = Quitter ?
confirm-body = Voulez-vous vraiment quitter ?
delete-title = Supprimer l'élément ?
delete-body = Cette action est irréversible.
delete = Supprimer
flavor-title = Choisissez un parfum
cancel = Annuler
vanilla = vanille
pistachio = pistache

# Files playground (docs/files.md)
files-caption = Sélecteurs de fichiers natifs. « Ouvrir » lit un fichier texte dans l'éditeur ; « Enregistrer » l'écrit.
files-placeholder = Saisissez du texte à enregistrer…
files-open = Ouvrir un fichier…
files-save = Enregistrer le fichier…
files-opened = Ouvert : { $name }

# Battery playground (docs/battery.md)
battery-refresh = Lire la batterie
battery-level = Niveau
battery-charging = En charge
battery-reading = Batterie : { $percent } · { $state }
battery-reading-none = Batterie : aucune API batterie sur cette plateforme

# Aire de jeu Capteurs (docs/sensors.md)
sensors-refresh = Lire les capteurs
sensor-accelerometer = Accéléromètre
sensor-gyroscope = Gyroscope
sensor-magnetometer = Magnétomètre
sensor-reading = x { $x } · y { $y } · z { $z } { $unit }
sensor-waiting = en attente du premier échantillon…
sensor-unavailable = indisponible sur cet appareil

# Aire de jeu Presse-papiers (docs/clipboard.md)
clipboard-caption = La part day-part-clipboard lit et écrit le presse-papiers système nativement.
clipboard-placeholder = Saisissez un texte à copier
clipboard-copy = Copier
clipboard-paste = Coller
clipboard-idle = Presse-papiers intact
clipboard-copied = Copié dans le presse-papiers système
clipboard-copy-failed = Échec de la copie (pas d'API presse-papiers ici)
clipboard-pasted = Collé depuis le presse-papiers système
clipboard-empty = Presse-papiers vide (ou illisible en arrière-plan)

# Aire de jeu Réseau (docs/network.md)
network-refresh = Lire le réseau
network-reading-online = En ligne · { $kind } · facturé : { $expensive }
network-reading-offline = Hors ligne
network-reading-none = Aucune API de connectivité sur cette plateforme

# Aire de jeu Média (docs/media.md)
media-play = Lecture
media-pause = Pause
media-load = Charger

# Aire de jeu Texte (typographie)
text-caption = Les styles sémantiques correspondent aux styles natifs et à l'échelle de texte d'accessibilité.
text-styles-header = Styles
text-weights-header = Graisses
text-styling-header = Gras et italique
text-colors-header = Couleur
text-custom-header = Tailles personnalisées
text-custom-note = Font.System(pt) — mis à l'échelle par la taille de texte d'accessibilité (Dynamic Type).
text-fonts-header = Polices embarquées
text-fonts-note = Font.Custom("Famille", pt) — fichiers du dossier fonts/ de l'application, embarqués par day build et résolus par nom de famille sur chaque plateforme.

# Aire de jeu Menus
menus-caption = Menus natifs — la barre de menus de l'application et les menus contextuels par élément — avec sous-menus imbriqués, raccourcis clavier et commandes d'édition standard.
menus-last = Dernière action :
menus-lifecycle = Dernière phase du cycle de vie :
menus-context-hint = Menu contextuel
menus-target = Clic droit ici (appui long sur mobile) pour un menu contextuel
menus-shortcut-hint = Les raccourcis clavier (⌘/Ctrl + touche) apparaissent dans la barre de menus et fonctionnent quand l'application est active — p. ex. Nouveau (N), Enregistrer (S), Recharger (R).

# --- day-part-haptics ---
nav-haptics = Haptique
haptics-supported-yes = Moteur haptique disponible sur cette plateforme
haptics-supported-no = Aucun moteur haptique sur cette plateforme (les boutons sont silencieux)
haptics-light = Léger
haptics-medium = Moyen
haptics-heavy = Fort
haptics-success = Succès
haptics-warning = Avertissement
haptics-error = Erreur
haptics-selection = Sélection
haptics-last = Dernier joué
haptics-none = Rien joué pour l'instant
haptics-last-played = Joué : { $style }

# --- day-part-prefs ---
nav-prefs = Préférences
prefs-caption = Conserver une chaîne entre les lancements avec day-part-prefs.
prefs-placeholder = Valeur à mémoriser
prefs-save = Enregistrer
prefs-load = Charger
prefs-clear = Effacer
prefs-idle = Saisissez une valeur, puis Enregistrer.
prefs-empty = (rien d'enregistré)
prefs-saved = Enregistré.
prefs-save-failed = Échec de l'enregistrement.
prefs-loaded = Chargé depuis le stockage.
prefs-missing = Rien d'enregistré pour l'instant.
prefs-cleared = Effacé.
prefs-value-label = Valeur enregistrée :

# --- bundled resources (§18.3) ---
nav-resources = Ressources
resources-caption = Une image chargée par nom depuis une ressource, avec accès aléatoire à des données embarquées.
resources-numbers = numbers.bin : { $len } octets, byte[100] = { $byte }
resources-greeting = greeting.txt : { $text }

# --- day-part-deviceinfo ---
nav-deviceinfo = Appareil
deviceinfo-model = Modèle : {$value}
deviceinfo-system = Système : {$name} {$version}
deviceinfo-simulator = Simulateur : {$value}
deviceinfo-yes = oui
deviceinfo-no = non
deviceinfo-refresh = Actualiser

# --- day-piece-activity ---
activity-animating = Animation
activity-on = En rotation
activity-off = Arrêté

# --- day-piece-searchfield ---
nav-search = Recherche
search-placeholder = Rechercher un fruit…
search-clear = Effacer

# --- day-piece-map ---
nav-map = Carte
map-caption = Une MKMapView native — plateformes Apple uniquement. Touchez un préréglage pour recentrer la carte en direct.
map-sf = San Francisco
map-nyc = New York

# — page tweaks (docs/tweaks.md) —
nav-tweaks = Tweaks
tweaks-intro = Les tweaks empaquetés configurent le composant natif derrière une pièce intégrée, par toolkit. Là où un tweak n'est pas couvert, il est sans effet — les pièces ci-dessous restent d'origine.
tweaks-stock = D'origine
tweaks-tweaked = Ajustée
tweaks-bezel-title = Biseau du bouton
tweaks-bezel-caption = day-tweak-button-bezel — AppKit uniquement : les constantes NSBezelStyle sur le vrai NSButton.
tweaks-selectable-title = Libellé sélectionnable
tweaks-selectable-caption = day-tweak-label-selectable — AppKit, GTK, Android : la sélection de texte native sur un libellé standard.
tweaks-selectable-text = Le texte de ce libellé peut être sélectionné et copié — essayez.
tweaks-ticks-title = Graduations du curseur
tweaks-ticks-caption = day-tweak-slider-tickmarks — AppKit, GTK, Android, Qt, WinUI, ArkUI : graduations natives, avec aimantation là où la plateforme la propose. Le curseur ajusté s'aimante ; celui d'origine glisse.
tweaks-ref-title = Vivacité du NativeRef
tweaks-ref-caption = Un NativeRef atteint le curseur ajusté après montage ; démontez-le et la référence se vide au lieu de pendre.
tweaks-ref-live = réf : vivante
tweaks-ref-cleared = réf : vidée

# — merged section pages (design overhaul) —
nav-canvas = Canevas et formes
nav-system = Appareil et capteurs
nav-services = Services système
controls-caption = Liaisons bidirectionnelles : chaque contrôle projette un signal de l'application.
controls-basics = Essentiels
controls-feedback = Retour visuel
canvas-caption = Formes, transformations, gestes et widgets composés — tous dessinés via le canevas.
canvas-gauge = Jauge canevas
shapes-interact-hint = Glissez le curseur pour pivoter, touchez le cercle pour recolorer, déplacez le carré violet.
system-caption = Les modules d'état de l'appareil : batterie, connectivité, capteurs et identité.
services-caption = Les modules « agir avec l'OS » : presse-papiers, préférences, haptique et fichiers.
subscribe-label = S'abonner

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = chocolat
size-small = Petit
size-medium = Moyen
size-large = Grand
fruit-apple = Pomme
fruit-banana = Banane
fruit-cherry = Cerise
fruit-date = Datte
fruit-elderberry = Sureau
list-row = Ligne { $n }
text-style-large-title = Grand titre
text-style-title = Titre
text-style-title2 = Titre 2
text-style-title3 = Titre 3
text-style-headline = En-tête
text-style-subheadline = Sous-en-tête
text-style-body = Corps
text-style-callout = Encadré
text-style-footnote = Note de bas de page
text-style-caption = Légende
text-style-caption2 = Légende 2
text-weight-ultralight = Ultra-fin
text-weight-light = Fin
text-weight-regular = Normal
text-weight-medium = Moyen
text-weight-semibold = Demi-gras
text-weight-bold = Gras
text-weight-heavy = Très gras
text-weight-black = Noir
text-bold = Texte gras
text-italic = Texte italique
text-bolditalic = Gras italique
text-emphasis-label = Emphase
color-red = Rouge
color-green = Vert
color-blue = Bleu
color-orange = Orange
